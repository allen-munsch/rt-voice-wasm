//! live-captions — native desktop binary: mic capture → STT → stdout JSON-lines.
//!
//! Reads config from ~/.config/rt-voice/config.toml, overridden by CLI flags.
//!
//! Usage:
//!   live-captions [--config PATH] [--model MODEL] [--wav-file FILE]
//!                 [--device NAME] [--speed 1.0] [--agent-hook CMD]
//!                 [--window-secs 3.0] [--step-secs 1.0]
//!                 [--list-devices] [--use-moonshine]

use std::sync::{Arc, Mutex, mpsc};
use std::io::Write;
use rt_voice_wasm::audio::{read_wav_i16, speedup};
use rt_voice_wasm::capture::MicCapture;
use rt_voice_wasm::config::RtVoiceConfig;
use rt_voice_wasm::engine::SttEngine;
use rt_voice_wasm::moonshine::MoonshineEngine;
use rt_voice_wasm::whisper::WhisperContext;
use rt_voice_wasm::stream::StreamingPipeline;

#[derive(serde::Serialize)]
struct CaptionEvent {
    event: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rtf: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    inference_ms: Option<f64>,
}

impl CaptionEvent {
    fn transcript(text: &str) -> Self {
        Self { event: "transcript".into(), text: Some(text.into()), rtf: None, inference_ms: None }
    }
    fn error(msg: &str) -> Self {
        Self { event: "error".into(), text: Some(msg.into()), rtf: None, inference_ms: None }
    }
    fn latency(rtf: f64, inference_ms: f64) -> Self {
        Self { event: "latency".into(), text: None, rtf: Some(rtf), inference_ms: Some(inference_ms) }
    }
    fn eos() -> Self {
        Self { event: "end_of_stream".into(), text: None, rtf: None, inference_ms: None }
    }
}

fn emit_json(event: &CaptionEvent) {
    let line = serde_json::to_string(event).unwrap();
    println!("{line}");
    let _ = std::io::stdout().flush();
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.contains(&"--list-devices".to_string()) {
        match MicCapture::list_devices() {
            Ok(devices) => {
                eprintln!("Input devices:");
                for d in &devices {
                    eprintln!("  {d}");
                }
            }
            Err(e) => {
                eprintln!("error listing devices: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

    let config = RtVoiceConfig::load(&args);
    let model_path = config.resolve_model_path();
    if model_path.is_empty() {
        eprintln!("no model found. pass --model PATH or install to /usr/share/rt-voice-wasm/models/");
        std::process::exit(1);
    }

    let use_moonshine = config.engine == "moonshine";
    let engine: Arc<Mutex<dyn SttEngine>> = if use_moonshine {
        Arc::new(Mutex::new(
            MoonshineEngine::init_from_dir(&model_path).unwrap_or_else(|e| {
                eprintln!("failed to create moonshine engine: {e}");
                std::process::exit(1);
            }),
        ))
    } else {
        Arc::new(Mutex::new(
            WhisperContext::init_from_file(&model_path).unwrap_or_else(|e| {
                eprintln!("failed to create whisper engine: {e}");
                std::process::exit(1);
            }),
        ))
    };

    // Agent hook (external command as stdin/stdout JSON)
    let mut agent_child: Option<std::process::Child> = None;
    let mut agent_stdin: Option<std::process::ChildStdin> = None;
    if let Some(ref hook) = config.agent_hook {
        let mut child = std::process::Command::new("sh")
            .args(["-c", hook])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .spawn()
            .unwrap_or_else(|e| {
                eprintln!("failed to start agent hook '{hook}': {e}");
                std::process::exit(1);
            });
        let stdin = child.stdin.take().unwrap();
        agent_stdin = Some(stdin);
        agent_child = Some(child);
    }

    let (sender, receiver) = mpsc::channel::<Vec<i16>>();

    let sample_rate: u32 = if let Some(ref wav) = config.wav_file {
        let (samples, rate) = read_wav_i16(wav).unwrap_or_else(|e| {
            eprintln!("error reading WAV: {e}");
            std::process::exit(1);
        });
        eprintln!("[live-captions] read {} samples at {} Hz from {}", samples.len(), rate, wav);
        let sender = sender.clone();
        std::thread::spawn(move || {
            let chunk_size = (rate as usize / 16).max(480);
            for chunk in samples.chunks(chunk_size) {
                let _ = sender.send(chunk.to_vec());
            }
            drop(sender);
        });
        rate
    } else {
        let cap = MicCapture::start_with_device(config.device.as_deref()).unwrap_or_else(|e| {
            eprintln!("error starting mic capture: {e}");
            std::process::exit(1);
        });
        eprintln!("[live-captions] capturing from default mic at 16 kHz");
        std::thread::spawn(move || {
            while let Some(chunk) = cap.recv() {
                if sender.send(chunk).is_err() {
                    break;
                }
            }
        });
        16000
    };

    let speed_factor = config.speed;
    let engine_name = if use_moonshine { "moonshine" } else { "whisper" };

    let worker = std::thread::spawn(move || {
        let mut pipeline = StreamingPipeline::with_params(
            sample_rate, speed_factor, config.window_secs, config.step_secs,
        );

        eprintln!("[live-captions] started — engine={engine_name} speed={speed_factor} window={}s step={}s",
            config.window_secs, config.step_secs);

        let mut full_transcript = String::new();
        for chunk in receiver {
            if let Some(window) = pipeline.push_frame(&chunk) {
                let sped = speedup(&window, pipeline.speed_factor());
                let result = engine.lock().unwrap().transcribe(&sped);

                match result {
                    Ok(texts) => {
                        let text: String = texts.join(" ");
                        let merged = pipeline.merge_overlap(&text);

                        if !merged.is_empty() {
                            full_transcript.push_str(&merged);
                            full_transcript.push(' ');

                            emit_json(&CaptionEvent::transcript(&merged));

                            // Send to agent hook if active
                            if let Some(ref mut stdin) = agent_stdin {
                                let msg = serde_json::json!({"event": "transcript", "text": &merged});
                                let _ = writeln!(stdin, "{}", serde_json::to_string(&msg).unwrap());
                            }
                        }
                    }
                    Err(e) => {
                        emit_json(&CaptionEvent::error(&e));
                    }
                }
            }
        }

        eprintln!("[live-captions] stream ended. full transcript: '{}'", full_transcript.trim());
        emit_json(&CaptionEvent::eos());
    });

    worker.join().unwrap();

    // Clean up agent process
    drop(agent_stdin);
    if let Some(mut child) = agent_child {
        match child.wait() {
            Ok(status) => eprintln!("[live-captions] agent hook exited with status: {status}"),
            Err(e) => eprintln!("[live-captions] agent hook wait error: {e}"),
        }
    }
}

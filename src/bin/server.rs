//! rt-voice-server — thin composer: wires transport + whisper + agent → call handler.
//!
//! Usage:
//!   rt-voice-server [--port 8080] [--speed 1.5] [--provider twilio|vapi|raw]
//!                   [--agent-hook 'command'] [--agent-rules builtin]
//!                   [--model path/to/model.bin]
//!
//! Providers:
//!   twilio — Twilio Media Streams (μ-law, base64, JSON events)
//!   vapi   — VAPI-compatible (same wire format as Twilio)
//!   raw    — Raw 16-bit PCM WebSocket (for custom integrations)
//!
//! Agents:
//!   builtin (default) — keyword rule matching via IntentRouter
//!   --agent-hook 'command' — external process agent (dirge-code, pi, etc.)

use rt_voice_wasm::agent::{default_rules, IntentRouter, ProcessAgent};
use rt_voice_wasm::call::{CallConfig, CallHandler};
use rt_voice_wasm::engine::SttEngine;
use rt_voice_wasm::transport::{AudioTransport, RawWsTransport, TwilioTransport};
use rt_voice_wasm::whisper::WhisperContext;

use std::sync::Arc;

use tokio::net::TcpListener;
use tokio_tungstenite::accept_async;

#[derive(Clone, Copy)]
enum Provider {
    Twilio,
    Vapi,
    Raw,
}

struct Config {
    model: String,
    port: u16,
    speed: f64,
    provider: Provider,
    agent_hook: Option<String>,
    greeting: String,
}

fn parse_args() -> Config {
    let args: Vec<String> = std::env::args().collect();
    let mut cfg = Config {
        model: "models/ggml-base.en-q5_1.bin".into(),
        port: 8080,
        speed: 1.0,
        provider: Provider::Twilio,
        agent_hook: None,
        greeting: CallConfig::default().greeting,
    };

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--model" => {
                i += 1;
                if i < args.len() {
                    cfg.model = args[i].clone();
                }
            }
            "--port" => {
                i += 1;
                if i < args.len() {
                    cfg.port = args[i].parse().unwrap_or(8080);
                }
            }
            "--speed" => {
                i += 1;
                if i < args.len() {
                    cfg.speed = args[i].parse().unwrap_or(1.0);
                }
            }
            "--provider" => {
                i += 1;
                if i < args.len() {
                    cfg.provider = match args[i].as_str() {
                        "twilio" => Provider::Twilio,
                        "vapi" => Provider::Vapi,
                        "raw" => Provider::Raw,
                        other => {
                            eprintln!("unknown provider '{other}', using twilio");
                            Provider::Twilio
                        }
                    };
                }
            }
            "--agent-hook" => {
                i += 1;
                if i < args.len() {
                    cfg.agent_hook = Some(args[i].clone());
                }
            }
            "--greeting" => {
                i += 1;
                if i < args.len() {
                    cfg.greeting = args[i].clone();
                }
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            _ => {
                eprintln!("unknown flag: {}", args[i]);
                i += 1;
            }
        }
        i += 1;
    }
    cfg
}

fn print_help() {
    eprintln!(
        r#"rt-voice-server — Whisper-powered real-time call transcription + agent routing

USAGE:
  rt-voice-server [FLAGS]

FLAGS:
  --model PATH     Whisper model path (default: models/ggml-tiny.en-q5_1.bin)
  --port N         WebSocket listen port (default: 8080)
  --speed FACTOR   Audio speedup factor (default: 1.0, e.g. 1.5 = 50% faster)
  --provider NAME  Audio transport: twilio, vapi, or raw (default: twilio)
  --agent-hook CMD External process for routing decisions
  --greeting TEXT  Greeting message sent on call connect
  --help, -h       Show this message

PROVIDERS:
  twilio  Twilio Media Streams — μ-law base64, JSON events
  vapi    VAPI-compatible — same wire format as Twilio
  raw     Raw 16-bit PCM over WebSocket — for custom integrations

AGENTS:
  builtin (default)  Keyword rule matching via IntentRouter
  --agent-hook CMD   External process: receives transcript lines on stdin,
                     writes JSON actions to stdout. Examples:
                       --agent-hook 'python3 my_router.py'
                       --agent-hook 'dirge-code route --format twilio'
                       --agent-hook 'pi --command route-call'

EXAMPLES:
  rt-voice-server
  rt-voice-server --speed 1.5 --provider raw --agent-hook 'python3 router.py'
  rt-voice-server --provider vapi --port 9090
"#
    );
}

fn build_agent(cfg: &Config) -> Box<dyn rt_voice_wasm::agent::Agent> {
    if let Some(ref hook) = cfg.agent_hook {
        match ProcessAgent::spawn(hook) {
            Ok(pa) => {
                eprintln!("[agent] using external process: {hook}");
                return Box::new(pa);
            }
            Err(e) => {
                eprintln!("[agent] failed to spawn '{hook}': {e}, falling back to builtin");
            }
        }
    }
    eprintln!("[agent] using builtin keyword router");
    Box::new(IntentRouter::new(default_rules()))
}

#[tokio::main]
async fn main() {
    let cfg = parse_args();

    eprintln!("[model] loading from {}...", cfg.model);
    let ctx: Arc<dyn SttEngine> = Arc::new(
        WhisperContext::init_from_file(&cfg.model).expect("failed to load whisper model"),
    );

    let call_cfg = CallConfig::default()
        .with_speed(cfg.speed)
        .with_greeting(&cfg.greeting);

    let addr = format!("0.0.0.0:{}", cfg.port);
    let listener = TcpListener::bind(&addr).await.expect("failed to bind");

    let provider_name = match cfg.provider {
        Provider::Twilio => "twilio",
        Provider::Vapi => "vapi",
        Provider::Raw => "raw",
    };
    eprintln!(
        "[server] listening on ws://{addr} (provider={provider_name}, speed={}x)",
        cfg.speed
    );
    eprintln!("[server] Twilio URL: ws://your-host:{}", cfg.port);

    while let Ok((stream, _)) = listener.accept().await {
        let ctx = Arc::clone(&ctx);
        let call_cfg = call_cfg.clone();
        let provider = cfg.provider;
        let agent_hook = cfg.agent_hook.clone();

        tokio::spawn(async move {
            let ws_stream = match accept_async(stream).await {
                Ok(ws) => ws,
                Err(e) => {
                    eprintln!("ws upgrade failed: {e}");
                    return;
                }
            };

            let transport: Box<dyn AudioTransport> = match provider {
                Provider::Twilio | Provider::Vapi => {
                    Box::new(TwilioTransport::new(ws_stream))
                }
                Provider::Raw => Box::new(RawWsTransport::new(ws_stream)),
            };

            let agent: Box<dyn rt_voice_wasm::agent::Agent> = if let Some(ref hook) = agent_hook
            {
                match ProcessAgent::spawn(hook) {
                    Ok(pa) => {
                        eprintln!("[agent] using external process: {hook}");
                        Box::new(pa)
                    }
                    Err(e) => {
                        eprintln!("[agent] spawn failed: {e}, using builtin");
                        Box::new(IntentRouter::new(default_rules()))
                    }
                }
            } else {
                Box::new(IntentRouter::new(default_rules()))
            };

            let mut handler = CallHandler::new(transport, ctx, agent, call_cfg);

            let name = match provider {
                Provider::Twilio => "twilio",
                Provider::Vapi => "vapi",
                Provider::Raw => "raw",
            };
            eprintln!("[call] connected ({name})");

            let transcript = handler.run().await;
            eprintln!("[call] disconnected");
            eprintln!("[transcript] {transcript}");
        });
    }
}

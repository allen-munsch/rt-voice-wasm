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
use rt_voice_wasm::moonshine::MoonshineEngine;
use rt_voice_wasm::parakeet::ParakeetEngine;
use rt_voice_wasm::transport::{AudioTransport, RawWsTransport, TwilioTransport};
use rt_voice_wasm::whisper::WhisperContext;

use std::sync::{Arc, Mutex};

use tokio::net::TcpListener;
use tokio_tungstenite::accept_async;

#[derive(Clone, Copy)]
enum Engine {
    Whisper,
    Parakeet,
    Moonshine,
}

#[derive(Clone, Copy)]
enum Provider {
    Twilio,
    Vapi,
    Raw,
}

struct Config {
    engine: Engine,
    model: String,
    port: u16,
    speed: f64,
    provider: Provider,
    agent_hook: Option<String>,
    agent_type: AgentType,
    greeting: String,
}

#[derive(Clone, Copy)]
enum AgentType {
    Builtin,
    Order,
}

fn parse_args() -> Config {
    let args: Vec<String> = std::env::args().collect();
    let mut cfg = Config {
        engine: Engine::Moonshine,
        model: "models/moonshine-tiny".into(),
        port: 8080,
        speed: 1.0,
        provider: Provider::Twilio,
        agent_hook: Some("./scripts/dirge-agent.sh".into()),
        agent_type: AgentType::Builtin,
        greeting: CallConfig::default().greeting,
    };

    let mut i = 1;
    let mut agent_flag_seen = false;
    let mut hook_flag_seen = false;
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
            "--engine" => {
                i += 1;
                if i < args.len() {
                    cfg.engine = match args[i].as_str() {
                        "whisper" => Engine::Whisper,
                        "parakeet" => Engine::Parakeet,
                        "moonshine" => Engine::Moonshine,
                        other => {
                            eprintln!("unknown engine '{other}', using moonshine");
                            Engine::Moonshine
                        }
                    };
                }
            }
            "--agent-hook" => {
                i += 1;
                if i < args.len() {
                    cfg.agent_hook = Some(args[i].clone());
                    hook_flag_seen = true;
                }
            }
            "--agent" => {
                i += 1;
                agent_flag_seen = true;
                if i < args.len() {
                    cfg.agent_type = match args[i].as_str() {
                        "builtin" => AgentType::Builtin,
                        "order" => AgentType::Order,
                        other => {
                            eprintln!("unknown agent '{other}', using builtin");
                            AgentType::Builtin
                        }
                    };
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
    // An explicit --agent (builtin|order) means the user wants a built-in
    // agent, so drop the convenience default hook unless --agent-hook was
    // also given explicitly (which keeps its documented precedence).
    if agent_flag_seen && !hook_flag_seen {
        cfg.agent_hook = None;
    }
    cfg
}

fn print_help() {
    eprintln!(
        r#"rt-voice-server — real-time call transcription + agent routing (Whisper / Parakeet)

USAGE:
  rt-voice-server [FLAGS]

FLAGS:
  --engine NAME    STT engine: moonshine, whisper, or parakeet (default: moonshine)
  --model PATH     Model path (default: models/moonshine-tiny for moonshine)
  --port N         WebSocket listen port (default: 8080)
  --speed FACTOR   Audio speedup factor (default: 1.0, e.g. 1.5 = 50% faster)
  --provider NAME  Audio transport: twilio, vapi, or raw (default: twilio)
  --agent-hook CMD External process for routing decisions
  --agent TYPE    Built-in agent: builtin (IntentRouter) or order (OrderFlow) [default: builtin]
  --greeting TEXT  Greeting message sent on call connect
  --help, -h       Show this message

PROVIDERS:
  twilio  Twilio Media Streams — μ-law base64, JSON events
  vapi    VAPI-compatible — same wire format as Twilio
  raw     Raw 16-bit PCM over WebSocket — for custom integrations

ENGINES:
  moonshine Moonshine (ONNX Runtime) — lowest latency, streaming-optimized (default)
  whisper  whisper.cpp (GGML) — 3s window, VAD gating, overlap merge
  parakeet parakeet.cpp (GGML) — cache-aware streaming, built-in <EOU>, RTFx ~27x vs whisper on CPU

AGENTS:
  builtin (default)  Keyword rule matching via IntentRouter
  --agent-hook CMD   External process: receives transcript lines on stdin,
                     writes JSON actions to stdout. Examples:
                       --agent-hook 'python3 my_router.py'
                       --agent-hook 'dirge-code route --format twilio'
                       --agent-hook 'pi --command route-call'

EXAMPLES:
  rt-voice-server
  rt-voice-server --engine parakeet --model models/parakeet_realtime_eou_120m-v1.gguf
  rt-voice-server --speed 1.5 --provider raw --agent-hook 'python3 router.py'
  rt-voice-server --provider vapi --port 9090
"#
    );
}

#[tokio::main]
async fn main() {
    let cfg = parse_args();

    eprintln!("[model] loading from {}...", cfg.model);
    let ctx: Arc<Mutex<dyn SttEngine>> = match cfg.engine {
        Engine::Whisper => Arc::new(Mutex::new(
            WhisperContext::init_from_file(&cfg.model).expect("failed to load whisper model"),
        )),
        Engine::Parakeet => Arc::new(Mutex::new(
            ParakeetEngine::init_from_file(&cfg.model).expect("failed to load parakeet model"),
        )),
        Engine::Moonshine => Arc::new(Mutex::new(
            MoonshineEngine::init_from_dir(&cfg.model).expect("failed to load moonshine model"),
        )),
    };

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
        "[server] listening on ws://{addr} (provider={provider_name}, engine={}, speed={}x)",
        match cfg.engine { Engine::Whisper => "whisper", Engine::Parakeet => "parakeet", Engine::Moonshine => "moonshine" },
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

            let agent: Arc<dyn rt_voice_wasm::agent::Agent> = if let Some(ref hook) = agent_hook
            {
                match ProcessAgent::spawn(hook) {
                    Ok(pa) => {
                        eprintln!("[agent] using external process: {hook}");
                        Arc::new(pa)
                    }
                    Err(e) => {
                        eprintln!("[agent] spawn failed: {e}, using builtin");
                        Arc::new(IntentRouter::new(default_rules()))
                    }
                }
            } else {
                match cfg.agent_type {
                    AgentType::Order => {
                        eprintln!("[agent] using order-flow state machine");
                        Arc::new(rt_voice_wasm::agent::OrderFlowAgent::new(
                            rt_voice_wasm::agent::default_menu(),
                        ))
                    }
                    AgentType::Builtin => Arc::new(IntentRouter::new(default_rules())),
                }
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

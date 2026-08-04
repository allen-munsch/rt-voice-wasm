//! WebSocket server for Twilio Media Streams → Whisper transcription.
//!
//! Usage: rt-voice-server [--model <path>] [--port <port>]
//!
//! Accepts Twilio Media Streams WebSocket connections, decodes μ-law audio,
//! runs the same whisper streaming pipeline as the browser path, and
//! sends transcripts back over the WebSocket.

use rt_voice_wasm::audio::decode_mulaw_8k_to_16k;
use rt_voice_wasm::stream::StreamingPipeline;
use rt_voice_wasm::whisper::WhisperContext;

use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio_tungstenite::accept_async;

struct CallSession {
    pipeline: StreamingPipeline,
    ctx: Arc<WhisperContext>,
    ws_tx: futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
        tokio_tungstenite::tungstenite::Message,
    >,
}

impl CallSession {
    async fn process_media(&mut self, payload_b64: &str) {
        let audio_bytes = match base64_decode(payload_b64) {
            Some(b) => b,
            None => return,
        };

        let pcm = decode_mulaw_8k_to_16k(&audio_bytes);

        if let Some(window) = self.pipeline.push_frame(&pcm) {
            match self.ctx.transcribe(&window) {
                Ok((segments, timing)) => {
                    let text: String = segments.iter()
                        .map(|s| s.text.as_str())
                        .collect::<Vec<_>>()
                        .join(" ");
                    let merged = self.pipeline.merge_overlap(&text);
                    if !merged.is_empty() {
                        let msg = json!({
                            "event": "transcript",
                            "text": merged,
                            "rtf": timing.total_ms / 1000.0 / (window.len() as f64 / 16000.0),
                        });
                        let _ = self.ws_tx
                            .send(tokio_tungstenite::tungstenite::Message::Text(
                                msg.to_string(),
                            ))
                            .await;
                    }
                }
                Err(e) => {
                    eprintln!("transcription error: {e}");
                }
            }
        }
    }
}

fn base64_decode(input: &str) -> Option<Vec<u8>> {
    use std::collections::HashMap;
    let alphabet: Vec<char> =
        "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
            .chars()
            .collect();
    let mut decode_map = HashMap::new();
    for (i, &c) in alphabet.iter().enumerate() {
        decode_map.insert(c, i as u8);
    }

    let input = input.trim_end_matches('=');
    let mut result = Vec::new();
    let chars: Vec<char> = input.chars().collect();

    for chunk in chars.chunks(4) {
        let vals: Vec<u8> = chunk.iter()
            .filter_map(|c| decode_map.get(c).copied())
            .collect();
        if vals.len() < 2 { break; }
        result.push((vals[0] << 2) | (vals[1] >> 4));
        if vals.len() > 2 {
            result.push((vals[1] << 4) | (vals[2] >> 2));
        }
        if vals.len() > 3 {
            result.push((vals[2] << 6) | vals[3]);
        }
    }
    Some(result)
}

async fn handle_connection(
    ws_stream: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    ctx: Arc<WhisperContext>,
) {
    let (ws_tx, mut ws_rx) = ws_stream.split();
    let pipeline = StreamingPipeline::new(16000);

    let mut session = CallSession {
        pipeline,
        ctx,
        ws_tx,
    };

    while let Some(Ok(msg)) = ws_rx.next().await {
        match msg {
            tokio_tungstenite::tungstenite::Message::Text(text) => {
                let v: Value = match serde_json::from_str(&text) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                match v["event"].as_str() {
                    Some("media") => {
                        if let Some(payload) = v["media"]["payload"].as_str() {
                            session.process_media(payload).await;
                        }
                    }
                    Some("start") => {
                        println!("stream started: {}",
                            v["streamSid"].as_str().unwrap_or("unknown"));
                    }
                    Some("stop") => {
                        println!("stream stopped");
                        break;
                    }
                    _ => {}
                }
            }
            tokio_tungstenite::tungstenite::Message::Close(_) => break,
            _ => {}
        }
    }
}

fn parse_args() -> (String, u16) {
    let args: Vec<String> = std::env::args().collect();
    let mut model = String::from("models/ggml-tiny.en-q5_1.bin");
    let mut port: u16 = 8080;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--model" => {
                i += 1;
                if i < args.len() {
                    model = args[i].clone();
                }
            }
            "--port" => {
                i += 1;
                if i < args.len() {
                    port = args[i].parse().unwrap_or(8080);
                }
            }
            _ => {}
        }
        i += 1;
    }
    (model, port)
}

#[tokio::main]
async fn main() {
    let (model_path, port) = parse_args();

    println!("Loading model from {model_path}...");
    let ctx = Arc::new(WhisperContext::init_from_file(&model_path)
        .expect("failed to load whisper model"));

    let addr = format!("0.0.0.0:{port}");
    let listener = TcpListener::bind(&addr).await.expect("failed to bind");

    println!("rt-voice-server listening on ws://{addr}");
    println!("Twilio Media Streams WebSocket URL: ws://your-host:{port}");

    while let Ok((stream, _)) = listener.accept().await {
        let ctx = ctx.clone();
        tokio::spawn(async move {
            match accept_async(stream).await {
                Ok(ws_stream) => {
                    println!("call connected");
                    handle_connection(ws_stream, ctx).await;
                    println!("call disconnected");
                }
                Err(e) => {
                    eprintln!("websocket upgrade failed: {e}");
                }
            }
        });
    }
}

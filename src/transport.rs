//! Pluggable audio transports — swap Twilio, VAPI, or raw WebSocket without
//! changing the call handler.

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::pin::Pin;
use tokio_tungstenite::tungstenite::Message;

/// Direction of an event: to the caller, to the system, or both.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Destination {
    Caller,
    System,
    Both,
}

/// An event sent over the transport.
#[derive(Debug, Clone)]
pub struct Event {
    pub kind: String,
    pub text: String,
    pub data: Value,
    pub to: Destination,
}

impl Event {
    pub fn transcript(text: &str) -> Self {
        Event {
            kind: "transcript".into(),
            text: text.to_string(),
            data: Value::Null,
            to: Destination::Both,
        }
    }

    pub fn agent_action(text: &str) -> Self {
        Event {
            kind: "agent_action".into(),
            text: text.to_string(),
            data: Value::Null,
            to: Destination::Caller,
        }
    }

    pub fn state(state: &str) -> Self {
        Event {
            kind: "state".into(),
            text: state.to_string(),
            data: Value::Null,
            to: Destination::System,
        }
    }

    pub fn error(text: &str) -> Self {
        Event {
            kind: "error".into(),
            text: text.to_string(),
            data: Value::Null,
            to: Destination::System,
        }
    }
}

/// Received audio chunk from any transport.
#[derive(Debug, Clone)]
pub struct AudioChunk {
    /// 16-bit PCM at 16 kHz
    pub pcm: Vec<i16>,
    /// Original sample rate before conversion
    pub original_rate: u32,
}

/// A transport that delivers audio chunks and sends events.
///
/// Implementations handle protocol specifics (Twilio Media Streams μ-law,
/// VAPI WebSocket, raw PCM, etc.) while the call handler stays protocol-agnostic.
pub trait AudioTransport: Send + 'static {
    /// Wait for the next audio chunk. Returns `None` when the stream ends.
    fn recv_audio(
        &mut self,
    ) -> Pin<Box<dyn std::future::Future<Output = Option<AudioChunk>> + Send + '_>>;

    /// Send an event to the remote side.
    fn send_event(
        &mut self,
        event: &Event,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + '_>>;

    /// Human-readable name for logging.
    fn name(&self) -> &str;
}

// ---------------------------------------------------------------------------
// Twilio Media Streams transport
// ---------------------------------------------------------------------------

enum TwilioMsg {
    Audio(Vec<i16>),
    Start { stream_sid: String },
    Stop,
    Close,
    Other,
}

pub struct TwilioTransport {
    ws_rx: futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    >,
    ws_tx: futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
        Message,
    >,
    stream_sid: Option<String>,
}

impl TwilioTransport {
    pub fn new(
        stream: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    ) -> Self {
        let (ws_tx, ws_rx) = stream.split();
        TwilioTransport {
            ws_rx,
            ws_tx,
            stream_sid: None,
        }
    }

    fn parse_message(&mut self, text: &str) -> TwilioMsg {
        let v: Value = match serde_json::from_str(text) {
            Ok(v) => v,
            Err(_) => return TwilioMsg::Other,
        };
        match v["event"].as_str() {
            Some("media") => {
                use crate::audio::decode_mulaw_8k_to_16k;
                if let Some(payload) = v["media"]["payload"].as_str() {
                    if let Some(raw) = base64_decode(payload) {
                        let pcm = decode_mulaw_8k_to_16k(&raw);
                        return TwilioMsg::Audio(pcm);
                    }
                }
                TwilioMsg::Other
            }
            Some("start") => {
                let sid = v["streamSid"].as_str().unwrap_or("unknown").to_string();
                self.stream_sid = Some(sid.clone());
                TwilioMsg::Start { stream_sid: sid }
            }
            Some("stop") => TwilioMsg::Stop,
            _ => TwilioMsg::Other,
        }
    }
}

impl AudioTransport for TwilioTransport {
    fn recv_audio(
        &mut self,
    ) -> Pin<Box<dyn std::future::Future<Output = Option<AudioChunk>> + Send + '_>> {
        Box::pin(async move {
            loop {
                let msg = self.ws_rx.next().await?;
                match msg {
                    Ok(Message::Text(text)) => match self.parse_message(&text) {
                        TwilioMsg::Audio(pcm) => {
                            return Some(AudioChunk {
                                pcm,
                                original_rate: 8000,
                            });
                        }
                        TwilioMsg::Start { .. } => continue,
                        TwilioMsg::Stop | TwilioMsg::Close => return None,
                        TwilioMsg::Other => continue,
                    },
                    Ok(Message::Close(_)) => return None,
                    Err(e) => {
                        eprintln!("Twilio WS error: {e}");
                        return None;
                    }
                    _ => continue,
                }
            }
        })
    }

    fn send_event(
        &mut self,
        event: &Event,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + '_>> {
        let payload = serde_json::json!({
            "event": event.kind,
            "text": event.text,
            "data": event.data,
        })
        .to_string();
        Box::pin(async move {
            self.ws_tx
                .send(Message::Text(payload))
                .await
                .map_err(|e| e.to_string())
        })
    }

    fn name(&self) -> &str {
        "twilio"
    }
}

// ---------------------------------------------------------------------------
// Raw WebSocket transport (16-bit PCM, no μ-law)
// ---------------------------------------------------------------------------

pub struct RawWsTransport {
    ws_rx: futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    >,
    ws_tx: futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
        Message,
    >,
}

impl RawWsTransport {
    pub fn new(
        stream: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    ) -> Self {
        let (ws_tx, ws_rx) = stream.split();
        RawWsTransport { ws_rx, ws_tx }
    }
}

impl AudioTransport for RawWsTransport {
    fn recv_audio(
        &mut self,
    ) -> Pin<Box<dyn std::future::Future<Output = Option<AudioChunk>> + Send + '_>> {
        Box::pin(async move {
            loop {
                let msg = self.ws_rx.next().await?;
                match msg {
                    Ok(Message::Binary(data)) => {
                        // Assume 16-bit LE PCM
                        let pcm: Vec<i16> = data
                            .chunks_exact(2)
                            .map(|c| i16::from_le_bytes([c[0], c[1]]))
                            .collect();
                        if !pcm.is_empty() {
                            return Some(AudioChunk {
                                pcm,
                                original_rate: 16000,
                            });
                        }
                    }
                    Ok(Message::Text(_)) => continue,
                    Ok(Message::Close(_)) => return None,
                    Err(e) => {
                        eprintln!("Raw WS error: {e}");
                        return None;
                    }
                    _ => continue,
                }
            }
        })
    }

    fn send_event(
        &mut self,
        event: &Event,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + '_>> {
        let payload = serde_json::json!({
            "event": event.kind,
            "text": event.text,
        })
        .to_string();
        Box::pin(async move {
            self.ws_tx
                .send(Message::Text(payload))
                .await
                .map_err(|e| e.to_string())
        })
    }

    fn name(&self) -> &str {
        "raw-ws"
    }
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

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
        let vals: Vec<u8> = chunk
            .iter()
            .filter_map(|c| decode_map.get(c).copied())
            .collect();
        if vals.len() < 2 {
            break;
        }
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

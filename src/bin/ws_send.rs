//! Tiny helper binary: send PCM audio chunks over WebSocket, print received JSON events.
//! Used by tts_harness.jolt to stream synthesized audio into rt-voice-server.
//!
//! Usage: ws-send <ws-url> <wav-path>  [--pad-secs N] [--chunk-ms N] [--timeout-s N]
//!
//! Reads a 16kHz mono 16-bit PCM WAV, pads to minimum duration, splits into
//! chunks, sends each as a binary WebSocket frame, then collects and prints
//! JSON events (one per line) until timeout or the server closes.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::time::timeout;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();

    let ws_url = args.get(1).expect("usage: ws-send <ws-url> <wav-path> [--pad-secs N] [--chunk-ms N] [--timeout-s N]");
    let wav_path = args.get(2).expect("usage: ws-send <ws-url> <wav-path> ...");

    let pad_secs: f64 = parse_flag(&args, "--pad-secs", 6.0);
    let chunk_ms: usize = parse_flag(&args, "--chunk-ms", 100);
    let timeout_s: f64 = parse_flag(&args, "--timeout-s", 25.0);

    let samples = read_wav_padded(wav_path, pad_secs);
    let chunk_samples = 16000 * chunk_ms / 1000;
    let chunks: Vec<Vec<u8>> = samples
        .chunks(chunk_samples)
        .filter(|c| c.len() == chunk_samples)
        .map(|c| {
            c.iter()
                .flat_map(|s| s.to_le_bytes())
                .collect::<Vec<u8>>()
        })
        .collect();

    let (ws, _) = connect_async(ws_url)
        .await
        .expect("websocket connect failed");

    let (mut write, mut read) = ws.split();

    for chunk in &chunks {
        write
            .send(Message::Binary(chunk.clone()))
            .await
            .expect("ws send failed");
    }

    drop(write);

    let deadline = Duration::from_secs_f64(timeout_s);
    let result: Result<(), _> = timeout(deadline, async {
        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    if text.contains("\"event\"") {
                        println!("{}", text);
                    }
                }
                Ok(Message::Close(_)) => break,
                Err(e) => {
                    eprintln!("ws error: {}", e);
                    break;
                }
                _ => {}
            }
        }
    })
    .await;

    if result.is_err() {
        eprintln!("timeout after {}s", timeout_s);
    }
}

fn read_wav_padded(path: &str, min_duration_secs: f64) -> Vec<i16> {
    let data = std::fs::read(path).expect("failed to read wav");
    assert!(&data[0..4] == b"RIFF", "not a WAV file");

    let data_start = find_chunk(&data, b"data").expect("no data chunk");
    let data_size = u32::from_le_bytes([
        data[data_start - 4],
        data[data_start - 3],
        data[data_start - 2],
        data[data_start - 1],
    ]) as usize;

    let raw = &data[data_start..data_start + data_size];
    let mut samples: Vec<i16> = raw
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect();

    let duration = samples.len() as f64 / 16000.0;
    if duration < min_duration_secs {
        let pad_samples = ((min_duration_secs - duration) * 16000.0) as usize;
        samples.extend(std::iter::repeat(0i16).take(pad_samples));
    }

    samples
}

fn find_chunk(data: &[u8], id: &[u8; 4]) -> Option<usize> {
    let mut pos = 12;
    while pos + 8 <= data.len() {
        if &data[pos..pos + 4] == id {
            return Some(pos + 8);
        }
        let size = u32::from_le_bytes([
            data[pos + 4],
            data[pos + 5],
            data[pos + 6],
            data[pos + 7],
        ]) as usize;
        pos += 8 + size + (size & 1);
    }
    None
}

fn parse_flag<T: std::str::FromStr>(args: &[String], name: &str, default: T) -> T {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

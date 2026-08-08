//! Deterministic tier-1 scenario replay tests. Loads every scenarios/*.json,
//! replays each tier-1 scenario's turns through CallHandler with MockTransport
//! and CannedEngine, and asserts expected agent_action events.
//!
//! No model, no network — just the Rust call path.

use rt_voice_wasm::agent::{Action, Agent, IntentRouter, OrderFlowAgent, Rule, default_menu};
use rt_voice_wasm::call::{CallConfig, CallHandler};
use rt_voice_wasm::engine::SttEngine;
use rt_voice_wasm::transport::{AudioChunk, AudioTransport, Event};

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Shared test fixtures (duplicated from call_handler.rs to keep tests modular)
// ---------------------------------------------------------------------------

struct MockTransport {
    chunks: Arc<Mutex<Vec<AudioChunk>>>,
    events: Arc<Mutex<Vec<Event>>>,
}

impl MockTransport {
    fn new(chunks: Vec<AudioChunk>) -> Self {
        Self {
            chunks: Arc::new(Mutex::new(chunks)),
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn events(&self) -> Vec<Event> {
        self.events.lock().unwrap().clone()
    }
}

impl AudioTransport for MockTransport {
    fn recv_audio(
        &mut self,
    ) -> Pin<Box<dyn std::future::Future<Output = Option<AudioChunk>> + Send + '_>> {
        let chunks = Arc::clone(&self.chunks);
        Box::pin(async move {
            // Pace chunks like real streaming audio so the in-flight agent
            // response is dispatched between turns instead of racing them.
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            let mut guard = chunks.lock().unwrap();
            if guard.is_empty() {
                None
            } else {
                Some(guard.remove(0))
            }
        })
    }

    fn send_event(
        &mut self,
        event: &Event,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + '_>> {
        let events = Arc::clone(&self.events);
        let ev = event.clone();
        Box::pin(async move {
            events.lock().unwrap().push(ev);
            Ok(())
        })
    }

    fn name(&self) -> &str {
        "mock"
    }
}

// ---------------------------------------------------------------------------
// CannedEngine: returns the scenario's speak phrase as the transcribed text,
// so the agent routes on exactly the words spoken.
// ---------------------------------------------------------------------------

struct CannedEngine {
    texts: Vec<String>,
    counter: Mutex<usize>,
}

impl SttEngine for CannedEngine {
    fn transcribe(&self, _samples: &[i16]) -> Result<Vec<String>, String> {
        let mut i = self.counter.lock().unwrap();
        let idx = (*i).min(self.texts.len() - 1);
        *i += 1;
        Ok(vec![self.texts[idx].clone()])
    }
}

fn canned_engine(texts: Vec<String>) -> Arc<Mutex<dyn SttEngine>> {
    Arc::new(Mutex::new(CannedEngine {
        counter: Mutex::new(0),
        texts,
    }))
}

// Generate a 440 Hz sine wave that passes VAD.
fn audio_chunk() -> AudioChunk {
    let freq = 440.0;
    let sample_rate = 16000.0;
    let amplitude = 5000i16;
    let samples: Vec<i16> = (0..64000)
        .map(|i| {
            let t = i as f64 / sample_rate;
            (amplitude as f64 * (2.0 * std::f64::consts::PI * freq * t).sin()) as i16
        })
        .collect();
    AudioChunk {
        pcm: samples,
        original_rate: 16000,
    }
}

// ---------------------------------------------------------------------------
// Scenario loader
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Deserialize)]
struct ScenarioFile {
    scenarios: Vec<Scenario>,
}

#[derive(Debug, serde::Deserialize)]
struct Scenario {
    name: String,
    description: String,
    #[serde(default = "default_tier")]
    tier: u32,
    turns: Vec<Turn>,
}

fn default_tier() -> u32 {
    1
}

#[derive(Debug, serde::Deserialize)]
struct Turn {
    speak: String,
    expect: Expect,
}

#[derive(Debug, serde::Deserialize)]
struct Expect {
    event: String,
    #[serde(default)]
    contains: String,
    #[serde(default)]
    not_contains: String,
}

fn load_scenarios_dir(dir: &str) -> Vec<(String, Scenario)> {
    let mut out = Vec::new();
    let paths = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return out,
    };
    for entry in paths.flatten() {
        let path = entry.path();
        if path.extension().map_or(false, |e| e == "json") {
            let data = std::fs::read_to_string(&path)
                .expect(&format!("failed to read {:?}", path));
            let sf: ScenarioFile = serde_json::from_str(&data)
                .expect(&format!("invalid JSON in {:?}", path));
            let fname = path.file_name().unwrap().to_string_lossy().to_string();
            for s in sf.scenarios {
                out.push((fname.clone(), s));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Agent builder: picks IntentRouter or OrderFlowAgent based on scenario file
// ---------------------------------------------------------------------------

fn make_agent(name: &str) -> Arc<dyn Agent> {
    match name {
        "order-taking.json" => Arc::new(OrderFlowAgent::new(default_menu())),
        _ => Arc::new(IntentRouter::new(rt_voice_wasm::agent::default_rules())),
    }
}

// ---------------------------------------------------------------------------
// Tier-1 replay tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn all_tier1_scenarios_pass() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/scenarios");
    let all = load_scenarios_dir(dir);
    assert!(!all.is_empty(), "expected scenarios/ directory with JSON files");

    let tier1: Vec<_> = all.into_iter().filter(|(_, s)| s.tier == 1).collect();
    assert!(!tier1.is_empty(), "expected at least one tier-1 scenario");

    for (file, scenario) in &tier1 {
        eprintln!("--- {} [{}] ---", scenario.name, file);

        // Build chunks and expected texts from turns
        let speaks: Vec<String> = scenario.turns.iter().map(|t| t.speak.clone()).collect();
        let chunks: Vec<AudioChunk> = speaks.iter().map(|_| audio_chunk()).collect();
        let transport = MockTransport::new(chunks);
        let events_ref = transport.events.clone();
        let engine = canned_engine(speaks);
        let agent = make_agent(file);

        let mut handler = CallHandler::new(
            Box::new(transport),
            engine,
            agent,
            CallConfig::default(),
        );

        let _transcript = handler.run().await;

        let events = events_ref.lock().unwrap().clone();

        // Check each turn's expect conditions
        let mut event_idx = 0usize;
        for (ti, turn) in scenario.turns.iter().enumerate() {
            let expect = &turn.expect;

            // Advance to find a matching event at or after current index
            let found = events[event_idx..].iter().enumerate().find(|(_, e)| {
                e.kind == expect.event
                    && (!expect.contains.is_empty() && e.text.to_lowercase().contains(&expect.contains.to_lowercase())
                        || expect.contains.is_empty())
            });

            match found {
                Some((rel_idx, _)) => {
                    event_idx += rel_idx;
                    let e = &events[event_idx];

                    // Check forbidden text
                    if !expect.not_contains.is_empty() {
                        assert!(
                            !e.text.to_lowercase().contains(&expect.not_contains.to_lowercase()),
                            "[{}] turn {}: event text '{}' contains forbidden '{}'",
                            scenario.name,
                            ti,
                            e.text,
                            expect.not_contains
                        );
                    }

                    event_idx += 1;
                }
                None => {
                    let kinds: Vec<_> = events.iter().map(|e| &e.kind).collect();
                    panic!(
                        "[{}] turn {}: no event matching kind='{}' contains='{}'. Got kinds: {:?}",
                        scenario.name, ti, expect.event, expect.contains, kinds
                    );
                }
            }
        }

        // No error events
        let errors: Vec<_> = events.iter().filter(|e| e.kind == "error").collect();
        assert!(
            errors.is_empty(),
            "[{}] unexpected error events: {:?}",
            scenario.name,
            errors.iter().map(|e| &e.text).collect::<Vec<_>>()
        );

        // Order scenarios should produce an order: agent_action
        if file == "order-taking.json" {
            let order_action = events.iter().find(|e| e.kind == "agent_action" && e.text.starts_with("order:"));
            match scenario.name.as_str() {
                "new-order" | "partial-slots" | "quantity-correction" | "add-item"
                | "remove-item" | "confirm-and-finalize" => {
                    assert!(order_action.is_some(),
                        "[{}] expected agent_action with 'order:' payload",
                        scenario.name);
                }
                "cancel" => {
                    // Expect cancelled message instead of order
                    let cancel = events.iter().find(|e| e.text.to_lowercase().contains("cancelled"));
                    assert!(cancel.is_some(),
                        "[{}] expected cancelled response", scenario.name);
                }
                _ => {}
            }
        }

        eprintln!("  OK");
    }
}

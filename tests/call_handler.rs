//! Integration tests for CallHandler — mock transport + mock engine → event sequence.
//!
//! Exercises the wire-level contract between AudioTransport, SttEngine, Agent, and CallHandler.

use rt_voice_wasm::agent::{Action, Agent, IntentRouter, Rule};
use rt_voice_wasm::call::{CallConfig, CallHandler};
use rt_voice_wasm::engine::SttEngine;
use rt_voice_wasm::transport::{AudioChunk, AudioTransport, Event};

use std::pin::Pin;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Mock transport: configurable input queue + event recording
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
            // Pace chunks like real streaming audio so in-flight agent
            // responses dispatch between turns instead of racing them.
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
// Mock engine: canned segments
// ---------------------------------------------------------------------------

struct CannedEngine {
    texts: Vec<Vec<String>>,
    counter: Mutex<usize>,
    fail: bool,
}

impl SttEngine for CannedEngine {
    fn transcribe(&self, _samples: &[i16]) -> Result<Vec<String>, String> {
        if self.fail {
            return Err("mock engine failure".into());
        }
        let mut i = self.counter.lock().unwrap();
        let idx = (*i).min(self.texts.len() - 1);
        *i += 1;
        Ok(self.texts[idx].clone())
    }
}

fn canned_engine(text: &str) -> Arc<Mutex<dyn SttEngine>> {
    Arc::new(Mutex::new(CannedEngine {
        texts: vec![vec![text.to_string()]],
        counter: Mutex::new(0),
        fail: false,
    }))
}

/// An engine that returns `text1` on the first call, `text2` on the second, etc.
fn cycling_engine(texts: Vec<&str>) -> Arc<Mutex<dyn SttEngine>> {
    Arc::new(Mutex::new(CannedEngine {
        texts: texts
            .into_iter()
            .map(|t| vec![t.to_string()])
            .collect(),
        counter: Mutex::new(0),
        fail: false,
    }))
}

fn failing_engine() -> Arc<Mutex<dyn SttEngine>> {
    Arc::new(Mutex::new(CannedEngine {
        texts: vec![],
        counter: Mutex::new(0),
        fail: true,
    }))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Return a chunk with enough 16kHz non-silent PCM to trigger one 3s window (48000 samples).
fn big_audio_chunk() -> AudioChunk {
    // Generate a 440 Hz sine wave — RMS ≈ amplitude / √2, well above the 0.01 threshold.
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

fn events_by_kind<'a>(events: &'a [Event], kind: &str) -> Vec<&'a Event> {
    events.iter().filter(|e| e.kind == kind).collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn greeting_events_arrive_first() {
    let transport = MockTransport::new(vec![big_audio_chunk()]);
    let events_ref = Arc::clone(&transport.events);
    let engine = canned_engine("hello world");
    let agent: Arc<dyn Agent> = Arc::new(IntentRouter::new(vec![]));

    let mut handler = CallHandler::new(
        Box::new(transport),
        engine,
        agent,
        CallConfig::default(),
    );

    let _transcript = handler.run().await;

    let events = events_ref.lock().unwrap().clone();
    let states: Vec<_> = events_by_kind(&events, "state")
        .iter()
        .map(|e| e.text.as_str())
        .collect();

    assert!(
        states.contains(&"greeting"),
        "expected greeting state event, got: {:?}",
        states
    );
}

#[tokio::test]
async fn transcript_event_arrives_after_window() {
    let transport = MockTransport::new(vec![big_audio_chunk()]);
    let events_ref = Arc::clone(&transport.events);
    let engine = canned_engine("hello world");
    let agent: Arc<dyn Agent> = Arc::new(IntentRouter::new(vec![]));

    let mut handler = CallHandler::new(
        Box::new(transport),
        engine,
        agent,
        CallConfig::default(),
    );

    handler.run().await;

    let events = events_ref.lock().unwrap().clone();
    let transcripts = events_by_kind(&events, "transcript");
    assert!(
        !transcripts.is_empty(),
        "expected at least one transcript event"
    );
    assert!(
        transcripts.iter().any(|e| e.text.contains("hello world")),
        "transcript should contain canned text"
    );
}

#[tokio::test]
async fn intent_router_triggers_agent_action() {
    // Two chunks: the first routes (Greeting→Routing) to an unmatched text that
    // produces no action, the second triggers the rule.
    let transport = MockTransport::new(vec![big_audio_chunk(), big_audio_chunk()]);
    let events_ref = Arc::clone(&transport.events);
    let engine = cycling_engine(vec!["hello for the first window", "transfer to agent now"]);
    let agent: Arc<dyn Agent> = Arc::new(IntentRouter::new(vec![Rule {
        triggers: vec!["transfer".into(), "agent".into()],
        action: Action::Transfer("agent".into()),
    }]));

    let mut handler = CallHandler::new(
        Box::new(transport),
        engine,
        agent,
        CallConfig::default(),
    );

    handler.run().await;

    let events = events_ref.lock().unwrap().clone();
    let actions = events_by_kind(&events, "agent_action");
    let has_transfer = actions.iter().any(|e| e.text.contains("transfer to agent"));
    assert!(
        has_transfer,
        "expected agent_action with 'transfer to agent', got: {:?}",
        actions.iter().map(|e| &e.text).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn full_transcript_event_on_disconnect() {
    let transport = MockTransport::new(vec![big_audio_chunk()]);
    let events_ref = Arc::clone(&transport.events);
    let engine = canned_engine("goodbye world");
    let agent: Arc<dyn Agent> = Arc::new(IntentRouter::new(vec![]));

    let mut handler = CallHandler::new(
        Box::new(transport),
        engine,
        agent,
        CallConfig::default(),
    );

    let transcript = handler.run().await;

    let events = events_ref.lock().unwrap().clone();
    let full = events_by_kind(&events, "full_transcript");
    assert_eq!(full.len(), 1, "expected exactly one full_transcript event");
    assert_eq!(full[0].text, transcript);
    assert!(
        transcript.contains("goodbye world"),
        "full transcript should contain engine output, got: '{}'",
        transcript
    );
}

#[tokio::test]
async fn engine_error_produces_error_event() {
    let transport = MockTransport::new(vec![big_audio_chunk()]);
    let events_ref = Arc::clone(&transport.events);
    let engine = failing_engine();
    let agent: Arc<dyn Agent> = Arc::new(IntentRouter::new(vec![]));

    let mut handler = CallHandler::new(
        Box::new(transport),
        engine,
        agent,
        CallConfig::default(),
    );

    handler.run().await;

    let events = events_ref.lock().unwrap().clone();
    let errors = events_by_kind(&events, "error");
    assert!(
        !errors.is_empty(),
        "expected at least one error event, got events: {:?}",
        events.iter().map(|e| &e.kind).collect::<Vec<_>>()
    );
}

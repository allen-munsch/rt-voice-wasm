//! Composable call handler: wires AudioTransport + StreamingPipeline + Whisper + Agent.
//!
//! Protocol-agnostic and agent-agnostic via boxed trait objects.

use crate::agent::{Action, Agent};
use crate::audio::speedup;
use crate::engine::SttEngine;
use crate::stream::StreamingPipeline;
use crate::transport::{AudioChunk, AudioTransport, Event};

use std::sync::{Arc, Mutex};

/// Call configuration.
#[derive(Clone)]
pub struct CallConfig {
    pub speed_factor: f64,
    pub greeting: String,
}

impl Default for CallConfig {
    fn default() -> Self {
        CallConfig {
            speed_factor: 1.0,
            greeting: "Hello, this is automated assistance. How can I help you today?".into(),
        }
    }
}

impl CallConfig {
    pub fn with_speed(mut self, factor: f64) -> Self {
        self.speed_factor = factor;
        self
    }

    pub fn with_greeting(mut self, greeting: impl Into<String>) -> Self {
        self.greeting = greeting.into();
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CallState {
    Greeting,
    Routing,
    Responding,
    Closing,
}

/// The call handler is transport-agnostic and agent-agnostic via boxed trait objects.
pub struct CallHandler {
    transport: Box<dyn AudioTransport>,
    pipeline: StreamingPipeline,
    ctx: Arc<Mutex<dyn SttEngine>>,
    agent: Arc<dyn Agent>,
    config: CallConfig,
    state: CallState,
    full_transcript: String,
}

impl CallHandler {
    pub fn new(
        transport: Box<dyn AudioTransport>,
        ctx: Arc<Mutex<dyn SttEngine>>,
        agent: Arc<dyn Agent>,
        config: CallConfig,
    ) -> Self {
        let pipeline = StreamingPipeline::with_speed(16000, config.speed_factor);
        CallHandler {
            transport,
            pipeline,
            ctx,
            agent,
            config,
            state: CallState::Greeting,
            full_transcript: String::new(),
        }
    }

    pub async fn run(&mut self) -> String {
        let _ = self.transport.send_event(&Event::state("greeting")).await;
        let _ = self
            .transport
            .send_event(&Event::agent_action(&format!("respond: {}", self.config.greeting)))
            .await;

        enum Flow {
            Agent(Action),
            Audio(AudioChunk),
            Closed,
        }

        // At most one agent request in flight; audio keeps flowing while it runs.
        let mut pending: Option<tokio::sync::oneshot::Receiver<Action>> = None;

        loop {
            let flow = tokio::select! {
                action = async {
                    match pending.as_mut() {
                        Some(rx) => rx.await.unwrap_or(Action::Continue),
                        None => std::future::pending::<Action>().await,
                    }
                } => Flow::Agent(action),
                chunk = self.transport.recv_audio() => match chunk {
                    Some(c) => Flow::Audio(c),
                    None => Flow::Closed,
                },
            };

            match flow {
                Flow::Agent(action) => {
                    pending = None;
                    self.dispatch_action(&action).await;
                }
                Flow::Closed => {
                    // Stream ended: finish any in-flight agent request before flushing.
                    if let Some(rx) = pending.take() {
                        if let Ok(action) = rx.await {
                            self.dispatch_action(&action).await;
                        }
                    }
                    break;
                }
                Flow::Audio(chunk) => {
                    eprintln!("[call] chunk {} samples, buf_len={}", chunk.pcm.len(), self.pipeline.buffer_len());
                    if let Some(window) = self.pipeline.push_frame(&chunk.pcm) {
                        eprintln!("[call] window {} samples, transcribing...", window.len());
                        let sped = speedup(&window, self.pipeline.speed_factor());

                        let ctx = Arc::clone(&self.ctx);
                        let sped_owned = sped;
                        match tokio::task::spawn_blocking(move || {
                            ctx.lock().unwrap().transcribe(&sped_owned)
                        })
                        .await
                        {
                            Ok(Err(e)) => {
                                eprintln!("[call] transcription error: {e}");
                                let _ = self.transport.send_event(&Event::error(&e)).await;
                            }
                            Err(join_err) => {
                                let msg = join_err.to_string();
                                eprintln!("[call] transcription task panicked: {msg}");
                                let _ = self.transport.send_event(&Event::error(&msg)).await;
                            }
                            Ok(Ok(texts)) => {
                                let text: String = texts.join(" ");
                                eprintln!("[call] transcribed {} segments: '{}'", texts.len(), &text[..text.len().min(80)]);
                                let merged = self.pipeline.merge_overlap(&text);

                                if !merged.is_empty() {
                                    eprintln!("[call] sending transcript: '{}'", &merged[..merged.len().min(50)]);
                                    self.full_transcript.push_str(&merged);
                                    self.full_transcript.push(' ');

                                    let _ = self
                                        .transport
                                        .send_event(&Event::transcript(&merged))
                                        .await;

                                    eprintln!("[call] transcript sent ok");
                                    self.on_transcript(&merged, &mut pending);
                                }
                            }
                        }
                    }
                }
            }
        }

        // Flush any remaining audio buffered in the pipeline
        if let Some(window) = self.pipeline.flush() {
            let sped = speedup(&window, self.pipeline.speed_factor());
            let ctx = Arc::clone(&self.ctx);
            let sped_owned = sped;
            if let Ok(Ok(texts)) = tokio::task::spawn_blocking(move || {
                ctx.lock().unwrap().transcribe(&sped_owned)
            })
            .await
            {
                let text: String = texts.join(" ");
                let merged = self.pipeline.merge_overlap(&text);
                if !merged.is_empty() {
                    self.full_transcript.push_str(&merged);
                    let _ = self.transport.send_event(&Event::transcript(&merged)).await;
                }
            }
        }

        let final_event = Event {
            kind: "full_transcript".into(),
            text: self.full_transcript.clone(),
            data: serde_json::Value::Null,
            to: crate::transport::Destination::System,
        };
        let _ = self.transport.send_event(&final_event).await;

        self.full_transcript.clone()
    }

    /// Advance the call state machine. In Routing with no request in flight,
    /// start a background agent request; the select loop in run() dispatches
    /// the response when it arrives, keeping audio flowing meanwhile.
    fn on_transcript(
        &mut self,
        text: &str,
        pending: &mut Option<tokio::sync::oneshot::Receiver<Action>>,
    ) {
        match self.state {
            CallState::Greeting | CallState::Routing => {
                if self.state == CallState::Greeting {
                    self.state = CallState::Routing;
                }
                if pending.is_some() {
                    return;
                }
                let agent = Arc::clone(&self.agent);
                let text_owned = text.to_string();
                let (tx, rx) = tokio::sync::oneshot::channel();
                tokio::spawn(async move {
                    let mut action = agent.route(&text_owned);
                    if matches!(action, Action::Continue) {
                        // ProcessAgent is non-blocking (route() returns Continue immediately,
                        // the LLM response arrives asynchronously on a background thread).
                        // Poll for up to 10 seconds for a non-Continue response.
                        for _ in 0..50 {
                            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                            action = agent.poll();
                            if !matches!(action, Action::Continue) {
                                break;
                            }
                        }
                    }
                    let _ = tx.send(action);
                });
                *pending = Some(rx);
            }
            CallState::Responding | CallState::Closing => {}
        }
    }

    async fn dispatch_action(&mut self, action: &Action) {
        match action {
            Action::Continue => {}
            Action::Respond(reply) => {
                // Stay in Routing so the conversation continues
                let _ = self
                    .transport
                    .send_event(&Event::agent_action(&format!("respond: {reply}")))
                    .await;
            }
            Action::Transfer(dest) => {
                self.state = CallState::Closing;
                let _ = self
                    .transport
                    .send_event(&Event::agent_action(&format!("transfer to {dest}")))
                    .await;
            }
            Action::Escalate(reason) => {
                self.state = CallState::Closing;
                let _ = self
                    .transport
                    .send_event(&Event::agent_action(&format!("escalate: {reason}")))
                    .await;
            }
            Action::Hangup => {
                self.state = CallState::Closing;
                let _ = self
                    .transport
                    .send_event(&Event::agent_action("hangup"))
                    .await;
            }
            Action::Order(payload) => {
                let _ = self
                    .transport
                    .send_event(&Event::agent_action(&format!("order: {payload}")))
                    .await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{default_rules, IntentRouter};

    #[test]
    fn call_config_defaults() {
        let cfg = CallConfig::default();
        assert_eq!(cfg.speed_factor, 1.0);
        assert!(cfg.greeting.contains("Hello"));
    }

    #[test]
    fn call_config_builder() {
        let cfg = CallConfig::default().with_speed(1.5).with_greeting("Hi!");
        assert_eq!(cfg.speed_factor, 1.5);
        assert_eq!(cfg.greeting, "Hi!");
    }

    #[test]
    fn call_state_routing_triggers_transfer() {
        let agent = IntentRouter::new(default_rules());
        let action = agent.route("I want to speak to an agent please");
        assert!(matches!(action, Action::Transfer(ref d) if d == "agent"));
    }
}

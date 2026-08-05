//! Composable call handler: wires AudioTransport + StreamingPipeline + Whisper + Agent.
//!
//! Protocol-agnostic and agent-agnostic via boxed trait objects.

use crate::agent::{Action, Agent};
use crate::audio::speedup;
use crate::engine::SttEngine;
use crate::stream::StreamingPipeline;
use crate::transport::{AudioTransport, Event};

use std::sync::Arc;

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
    ctx: Arc<dyn SttEngine>,
    agent: Box<dyn Agent>,
    config: CallConfig,
    state: CallState,
    full_transcript: String,
}

impl CallHandler {
    pub fn new(
        transport: Box<dyn AudioTransport>,
        ctx: Arc<dyn SttEngine>,
        agent: Box<dyn Agent>,
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

        while let Some(chunk) = self.transport.recv_audio().await {
            if let Some(window) = self.pipeline.push_frame(&chunk.pcm) {
                let sped = speedup(&window, self.pipeline.speed_factor());

                match self.ctx.transcribe(&sped) {
                    Ok(texts) => {
                        let text: String = texts.join(" ");
                        let merged = self.pipeline.merge_overlap(&text);

                        if !merged.is_empty() {
                            self.full_transcript.push_str(&merged);
                            self.full_transcript.push(' ');

                            let _ = self
                                .transport
                                .send_event(&Event::transcript(&merged))
                                .await;

                            self.handle_action(&merged).await;
                        }
                    }
                    Err(e) => {
                        eprintln!("[call] transcription error: {e}");
                        let _ = self.transport.send_event(&Event::error(&e)).await;
                    }
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

    async fn handle_action(&mut self, text: &str) {
        match self.state {
            CallState::Greeting => {
                self.state = CallState::Routing;
            }
            CallState::Routing => {
                let action = self.agent.route(text);
                match &action {
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
                }
            }
            CallState::Responding | CallState::Closing => {}
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

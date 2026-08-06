//! Lightweight agent system for phone call routing.
//!
//! Three agent types plug into the call handler:
//! - `IntentRouter` — keyword/phrase matching with configurable rules
//! - `FnAgent` — closure-based for inline logic
//! - `ProcessAgent` — spawns an external process (dirge-code, pi, shell script)

use std::io::Write;
use std::sync::mpsc::{self, Receiver};

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Action {
    Respond(String),
    Transfer(String),
    Escalate(String),
    Hangup,
    Continue,
}

/// The Agent trait: any type that can route a transcript to an Action.
pub trait Agent: Send + Sync {
    fn route(&self, text: &str) -> Action;

    /// Non-blocking poll for an already-pending action. Default returns Continue.
    fn poll(&self) -> Action {
        Action::Continue
    }
}

/// A single routing rule: trigger phrases → action.
#[derive(Clone)]
pub struct Rule {
    pub triggers: Vec<String>,
    pub action: Action,
}

/// Keyword/phrase matcher — first-match-wins over trigger list.
pub struct IntentRouter {
    rules: Vec<Rule>,
}

impl IntentRouter {
    pub fn new(rules: Vec<Rule>) -> Self {
        IntentRouter { rules }
    }
}

impl Agent for IntentRouter {
    fn route(&self, text: &str) -> Action {
        let lower = text.to_lowercase();
        for rule in &self.rules {
            for trigger in &rule.triggers {
                if lower.contains(&trigger.to_lowercase()) {
                    return rule.action.clone();
                }
            }
        }
        Action::Continue
    }
}

/// An agent backed by a closure — useful for inline logic or wrapping a channel.
pub struct FnAgent<F: Fn(&str) -> Action + Send + Sync> {
    f: F,
}

impl<F: Fn(&str) -> Action + Send + Sync> FnAgent<F> {
    pub fn new(f: F) -> Self {
        FnAgent { f }
    }
}

impl<F: Fn(&str) -> Action + Send + Sync> Agent for FnAgent<F> {
    fn route(&self, text: &str) -> Action {
        (self.f)(text)
    }
}

/// Spawns an external process for routing decisions.
///
/// The process receives transcript lines on stdin (one per line) and must
/// write a JSON action to stdout for each. Stderr is logged.
///
/// Expected JSON format on stdout:
/// ```json
/// {"Respond": "reply text"}
/// {"Transfer": "agent"}
/// {"Escalate": "reason"}
/// "Hangup"
/// "Continue"
/// ```
///
/// Example: `rt-voice-server --agent-hook 'python3 my_router.py'`
/// where `my_router.py` reads stdin and writes JSON actions to stdout.
pub struct ProcessAgent {
    stdin: std::sync::Mutex<std::process::ChildStdin>,
    rx: std::sync::Mutex<Receiver<Action>>,
    _child: std::sync::Mutex<std::process::Child>,
}

impl ProcessAgent {
    /// Spawn a command. The command string is split by whitespace; use quotes
    /// for multi-word args (e.g. `--agent-hook 'python3 my_script.py --flag'`).
    ///
    /// A background thread reads stdout lines and pushes parsed Actions into an
    /// mpsc channel. `route()` does non-blocking `try_recv()` so the call handler
    /// never blocks waiting for the LLM.
    pub fn spawn(cmd: &str) -> Result<Self, String> {
        let parts: Vec<&str> = shell_words(cmd);
        let mut child = std::process::Command::new(parts[0])
            .args(&parts[1..])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .spawn()
            .map_err(|e| format!("failed to spawn '{cmd}': {e}"))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "no stdin".to_string())?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "no stdout".to_string())?;

        let (tx, rx) = mpsc::channel();

        std::thread::spawn(move || {
            use std::io::{BufRead, BufReader};
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                match line {
                    Ok(l) => {
                        let action = Self::parse_action(&l);
                        if tx.send(action).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(ProcessAgent {
            stdin: std::sync::Mutex::new(stdin),
            rx: std::sync::Mutex::new(rx),
            _child: std::sync::Mutex::new(child),
        })
    }
}

impl Agent for ProcessAgent {
    fn route(&self, text: &str) -> Action {
        // Write transcript to child stdin
        if let Ok(mut stdin) = self.stdin.lock() {
            let _ = writeln!(stdin, "{text}");
            let _ = stdin.flush();
        }

        // Non-blocking: return any pending action from the background reader
        if let Ok(rx) = self.rx.lock() {
            rx.try_recv().unwrap_or(Action::Continue)
        } else {
            Action::Continue
        }
    }

    fn poll(&self) -> Action {
        if let Ok(rx) = self.rx.lock() {
            if let Ok(action) = rx.try_recv() {
                return action;
            }
        }
        Action::Continue
    }
}

impl ProcessAgent {
    fn parse_action(line: &str) -> Action {
        let line = line.trim();
        if line.is_empty() {
            return Action::Continue;
        }

        // Try outer wrapper first (dirge --output-format json)
        if let Ok(wrapper) = serde_json::from_str::<serde_json::Value>(line) {
            if let Some(inner) = wrapper.get("result").and_then(|v| v.as_str()) {
                return Self::parse_inner(inner);
            }
        }

        // Try raw action JSON directly
        Self::parse_inner(line)
    }

    fn parse_inner(json: &str) -> Action {
        let json = json.trim();
        if let Ok(obj) = serde_json::from_str::<serde_json::Value>(json) {
            if let Some(reply) = obj.get("Respond").and_then(|v| v.as_str()) {
                return Action::Respond(reply.to_string());
            }
            if let Some(dest) = obj.get("Transfer").and_then(|v| v.as_str()) {
                return Action::Transfer(dest.to_string());
            }
            if let Some(reason) = obj.get("Escalate").and_then(|v| v.as_str()) {
                return Action::Escalate(reason.to_string());
            }
            if obj.get("Hangup").is_some() {
                return Action::Hangup;
            }
        }
        Action::Continue
    }

    /// Poll for an already-pending action without writing to stdin.
    /// Returns Continue if nothing is ready yet.
    pub fn poll(&self) -> Action {
        if let Ok(rx) = self.rx.lock() {
            if let Ok(action) = rx.try_recv() {
                return action;
            }
        }
        Action::Continue
    }
}

impl Drop for ProcessAgent {
    fn drop(&mut self) {
        let _ = self._child.lock().ok().and_then(|mut c| c.kill().ok());
    }
}

fn shell_words(cmd: &str) -> Vec<&str> {
    cmd.split_whitespace().collect()
}

/// Pre-built phone bank routing configuration.
pub fn default_rules() -> Vec<Rule> {
    vec![
        Rule {
            triggers: vec![
                "what can you do".into(),
                "what do you do".into(),
                "capabilities".into(),
                "who are you".into(),
                "what are you".into(),
                "how can you help".into(),
                "what can i ask".into(),
            ],
            action: Action::Respond(
                "I'm the automated receptionist. I can answer questions, \
                 route you to the right department, transfer you to a live \
                 agent, or escalate issues to a supervisor. How can I help?".into(),
            ),
        },
        Rule {
            triggers: vec![
                "agent".into(), "representative".into(), "human".into(),
                "person".into(), "operator".into(), "speak to someone".into(),
            ],
            action: Action::Transfer("agent".into()),
        },
        Rule {
            triggers: vec![
                "support".into(), "help".into(), "issue".into(),
                "problem".into(), "broken".into(), "not working".into(),
                "error".into(), "bug".into(),
            ],
            action: Action::Transfer("support".into()),
        },
        Rule {
            triggers: vec![
                "president".into(), "manager".into(), "supervisor".into(),
                "escalate".into(), "complaint".into(),
            ],
            action: Action::Escalate("caller requested supervisor".into()),
        },
        Rule {
            triggers: vec![
                "goodbye".into(), "bye".into(), "hang up".into(),
                "that's all".into(), "thank you".into(), "thanks".into(),
            ],
            action: Action::Respond("Thank you for calling. Goodbye.".into()),
        },
        Rule {
            triggers: vec![
                "yes".into(), "yeah".into(), "yep".into(), "correct".into(),
            ],
            action: Action::Respond("Great, let me help you with that.".into()),
        },
        Rule {
            triggers: vec![
                "no".into(), "nope".into(), "not".into(),
            ],
            action: Action::Respond("Okay, let me try something else.".into()),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_agent_request() {
        let router = IntentRouter::new(default_rules());
        let result = router.route("I want to speak to an agent please");
        assert!(matches!(result, Action::Transfer(ref d) if d == "agent"));
    }

    #[test]
    fn route_support() {
        let router = IntentRouter::new(default_rules());
        let result = router.route("my computer is broken and not working");
        assert!(matches!(result, Action::Transfer(ref d) if d == "support"));
    }

    #[test]
    fn route_escalate() {
        let router = IntentRouter::new(default_rules());
        let result = router.route("I need to speak to your manager right now");
        assert!(matches!(result, Action::Escalate(_)));
    }

    #[test]
    fn route_continue_on_unmatched() {
        let router = IntentRouter::new(default_rules());
        let result = router.route("the weather is nice today");
        assert!(matches!(result, Action::Continue));
    }

    #[test]
    fn route_goodbye() {
        let router = IntentRouter::new(default_rules());
        let result = router.route("thank you so much goodbye");
        assert!(matches!(result, Action::Respond(_)));
    }

    #[test]
    fn process_agent_parse_raw_action() {
        assert!(matches!(
            ProcessAgent::parse_action("{\"Transfer\": \"support\"}"),
            Action::Transfer(ref d) if d == "support"
        ));
    }

    #[test]
    fn process_agent_with_dirge_wrapper() {
        let agent =
            ProcessAgent::spawn("./scripts/dirge-agent.sh").expect("spawn wrapper");

        // Send transcript; route() is non-blocking now
        let result = agent.route("I want to speak to an agent please");
        // First call returns Continue (LLM hasn't responded yet)
        assert!(matches!(result, Action::Continue));

        // Poll for the LLM response (up to 30s)
        for _ in 0..60 {
            std::thread::sleep(std::time::Duration::from_millis(500));
            let action = agent.route("");
            if !matches!(action, Action::Continue) {
                assert!(matches!(action, Action::Transfer(ref d) if d == "agent"));
                return;
            }
        }
        panic!("dirge agent did not respond within 30s");
    }

    #[test]
    fn process_agent_with_dirge_coding_agent() {
        let agent =
            ProcessAgent::spawn("./scripts/dirge-coding-agent.sh").expect("spawn coding agent");

        let result = agent.route("how many test files are in the tests directory");
        assert!(matches!(result, Action::Continue));

        for _ in 0..60 {
            std::thread::sleep(std::time::Duration::from_millis(500));
            let action = agent.route("");
            if !matches!(action, Action::Continue) {
                // Should get a Respond action with test file info
                assert!(matches!(action, Action::Respond(_)));
                return;
            }
        }
        panic!("dirge coding agent did not respond within 30s");
    }

    #[test]
    fn process_agent_parse_dirge_wrapper() {
        let json = r#"{"type":"result","result":"{\"Respond\": \"hello\"}","duration_ms":100}"#;
        assert!(matches!(
            ProcessAgent::parse_action(json),
            Action::Respond(ref r) if r == "hello"
        ));
    }

    #[test]
    fn process_agent_parse_hangup() {
        assert!(matches!(
            ProcessAgent::parse_action("{\"Hangup\": null}"),
            Action::Hangup
        ));
    }

    #[test]
    fn process_agent_parse_empty_returns_continue() {
        assert!(matches!(
            ProcessAgent::parse_action(""),
            Action::Continue
        ));
    }
}

//! Lightweight agent system for phone call routing.
//!
//! Three agent types plug into the call handler:
//! - `IntentRouter` — keyword/phrase matching with configurable rules
//! - `FnAgent` — closure-based for inline logic
//! - `ProcessAgent` — spawns an external process (dirge-code, pi, shell script)

use std::io::Write;

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
    child: std::sync::Mutex<std::process::Child>,
    stdin: std::sync::Mutex<std::process::ChildStdin>,
}

impl ProcessAgent {
    /// Spawn a command. The command string is split by whitespace; use quotes
    /// for multi-word args (e.g. `--agent-hook 'python3 my_script.py --flag'`).
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

        Ok(ProcessAgent {
            child: std::sync::Mutex::new(child),
            stdin: std::sync::Mutex::new(stdin),
        })
    }
}

impl Agent for ProcessAgent {
    fn route(&self, text: &str) -> Action {
        // Send transcript to the process
        {
            let mut stdin = match self.stdin.lock() {
                Ok(g) => g,
                Err(_) => return Action::Continue,
            };
            let _ = writeln!(stdin, "{text}");
            let _ = stdin.flush();
        }

        // We don't synchronously wait for a response — that would block.
        // In practice, the call handler polls. For a synchronous agent hook,
        // we return Continue and the external process feeds decisions via
        // a side-channel (file, socket, HTTP). For a lightweight built-in
        // process, override this with tokio-based async I/O.
        //
        // The intent: dirge-code/dirge or pi reads context from stdin and
        // writes decisions. The call handler accumulates transcript; the
        // external process has the full picture.
        Action::Continue
    }
}

impl Drop for ProcessAgent {
    fn drop(&mut self) {
        let _ = self.child.lock().ok().and_then(|mut c| c.kill().ok());
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
}

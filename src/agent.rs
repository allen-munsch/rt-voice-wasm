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
    /// Emit a structured order payload (item, quantity, size, etc.).
    Order(serde_json::Value),
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
        let words: Vec<&str> = lower.split_whitespace().collect();
        for rule in &self.rules {
            for trigger in &rule.triggers {
                let trigger_lower = trigger.to_lowercase();
                if trigger_contains(&lower, &trigger_lower, &words) {
                    return rule.action.clone();
                }
            }
        }
        // Catch-all so an unmatched utterance gets a prompt reply, not dead air.
        Action::Respond("I didn't quite catch that. You can ask for a live agent, support, or a manager.".into())
    }
}

/// Match a trigger against the utterance. Single-word triggers match whole
/// words only, stripping punctuation so "agent!" matches "agent";
/// multi-word triggers use substring matching so phrases like
/// "not working" still work in "it is not working".
fn trigger_contains(text: &str, trigger: &str, words: &[&str]) -> bool {
    if trigger.contains(' ') {
        text.contains(trigger)
    } else {
        words.iter().any(|w| {
            let trimmed = w.trim_matches(|c: char| !c.is_alphanumeric());
            trimmed == trigger
        })
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

/// Deterministic order-flow state machine: slot fill → confirm → finalize.
///
/// Handles new orders, corrections ("actually make it three"), add/remove
/// items, confirm, cancel, and unknown-item clarification.
pub struct OrderFlowAgent {
    state: std::sync::Mutex<OrderState>,
    menu: Vec<String>,
}

#[derive(Debug, Clone)]
struct OrderState {
    items: Vec<OrderItem>,
    phase: OrderPhase,
}

#[derive(Debug, Clone)]
struct OrderItem {
    name: String,
    quantity: u32,
    size: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
enum OrderPhase {
    AwaitingItem,
    AwaitingSize,
    AwaitingConfirmation,
    Confirmed,
    Cancelled,
}

impl OrderFlowAgent {
    pub fn new(menu: Vec<String>) -> Self {
        OrderFlowAgent {
            state: std::sync::Mutex::new(OrderState {
                items: Vec::new(),
                phase: OrderPhase::AwaitingItem,
            }),
            menu,
        }
    }

    fn find_item(&self, text: &str) -> Option<String> {
        let lower = text.to_lowercase();
        self.menu.iter().find(|item| lower.contains(&item.to_lowercase())).cloned()
    }
}

impl Agent for OrderFlowAgent {
    fn route(&self, text: &str) -> Action {
        let lower = text.to_lowercase();
        let mut state = self.state.lock().unwrap();

        // Cancel anywhere
        if lower.contains("cancel") {
            state.phase = OrderPhase::Cancelled;
            return Action::Respond("Order cancelled. Is there anything else?".into());
        }

        // Escalate to a human anywhere in the flow
        if lower.contains("manager")
            || lower.contains("supervisor")
            || lower.contains("escalate")
            || lower.contains("speak to a person")
        {
            return Action::Escalate("manager".into());
        }

        match &state.phase {
            OrderPhase::AwaitingItem => {
                // Check for an existing item being modified
                let is_correction = lower.contains("actually") || lower.contains("make it") || lower.contains("change");
                let is_add = lower.contains("add") && !lower.contains("address");
                let is_remove = lower.contains("remove");

                if state.items.is_empty() || is_correction || is_add || is_remove {
                    if is_remove && !state.items.is_empty() {
                        state.items.pop();
                        if state.items.is_empty() {
                            return Action::Respond("Removed. What would you like to order?".into());
                        }
                        return Action::Respond(format!(
                            "Removed. Your order now has {}. Confirm?",
                            describe_items(&state.items)
                        ));
                    }

                    if let Some(item) = self.find_item(text) {
                        let qty = parse_quantity(text).unwrap_or(1);
                        // Check size
                        let size = if lower.contains("large") { Some("large".into()) }
                            else if lower.contains("medium") { Some("medium".into()) }
                            else if lower.contains("small") { Some("small".into()) }
                            else { None };

                        if is_add && !state.items.is_empty() {
                            state.items.push(OrderItem { name: item, quantity: qty, size });
                            state.phase = OrderPhase::AwaitingConfirmation;
                            return Action::Respond(format!(
                                "Added. Your order now has {}. Confirm?",
                                describe_items(&state.items)
                            ));
                        }

                        state.items = vec![OrderItem { name: item, quantity: qty, size: size.clone() }];

                        if size.is_none() {
                            state.phase = OrderPhase::AwaitingSize;
                            return Action::Respond("What size — small, medium, or large?".into());
                        }
                        state.phase = OrderPhase::AwaitingConfirmation;
                        return Action::Respond(format!(
                            "{} {}. Confirm with yes or make changes.",
                            describe_items(&state.items),
                            if state.items.len() == 1 { "— is that correct?" } else { "" }
                        ));
                    }

                    return Action::Respond("I don't have that on the menu. We have coffee, latte, cappuccino, hot chocolate, tea, and muffins. What would you like?".into());
                }
            }
            OrderPhase::AwaitingSize => {
                if let Some(size) = if lower.contains("large") { Some("large") }
                    else if lower.contains("medium") { Some("medium") }
                    else if lower.contains("small") { Some("small") }
                    else { None }
                {
                    for item in &mut state.items {
                        if item.size.is_none() {
                            item.size = Some(size.to_string());
                        }
                        if let Some(qty) = parse_quantity(&lower) {
                            item.quantity = qty;
                        }
                    }
                    state.phase = OrderPhase::AwaitingConfirmation;
                    return Action::Respond(format!(
                        "{}. Confirm?",
                        describe_items(&state.items)
                    ));
                }
                // Re-specify item in this phase
                if let Some(item) = self.find_item(text) {
                    let qty = parse_quantity(text).unwrap_or(1);
                    state.items = vec![OrderItem { name: item, quantity: qty, size: None }];
                    return Action::Respond(format!("Updated to {}. What size?", describe_items(&state.items)));
                }
            }
            OrderPhase::AwaitingConfirmation => {
                let is_add = lower.contains("add") && !lower.contains("address");
                let is_remove = lower.contains("remove");

                if is_remove && !state.items.is_empty() {
                    state.items.pop();
                    if state.items.is_empty() {
                        return Action::Respond("Removed. What would you like to order?".into());
                    }
                    return Action::Respond(format!(
                        "Removed. Your order now has {}. Confirm?",
                        describe_items(&state.items)
                    ));
                }

                if is_add {
                    if let Some(item) = self.find_item(text) {
                        let qty = parse_quantity(text).unwrap_or(1);
                        state.items.push(OrderItem { name: item, quantity: qty, size: None });
                        return Action::Respond(format!(
                            "Added. Your order now has {}. Confirm?",
                            describe_items(&state.items)
                        ));
                    }
                }

                if lower.contains("yes") || lower.contains("correct") || lower.contains("yeah") || lower.contains("yep") {
                    state.phase = OrderPhase::Confirmed;
                    let payload = serde_json::json!({
                        "items": state.items.iter().map(|i| serde_json::json!({
                            "item": i.name,
                            "quantity": i.quantity,
                            "size": i.size,
                        })).collect::<Vec<_>>(),
                    });
                    return Action::Order(payload);
                }
                // Handle size specification
                if let Some(size) = if lower.contains("large") { Some("large") }
                    else if lower.contains("medium") { Some("medium") }
                    else if lower.contains("small") { Some("small") }
                    else { None }
                {
                    for item in &mut state.items {
                        if item.size.is_none() {
                            item.size = Some(size.to_string());
                        }
                    }
                    return Action::Respond(format!(
                        "{}. Confirm?",
                        describe_items(&state.items)
                    ));
                }
                // Handle re-specifying item
                if let Some(item) = self.find_item(text) {
                    let qty = parse_quantity(text).unwrap_or(1);
                    state.items = vec![OrderItem {
                        name: item,
                        quantity: qty,
                        size: None,
                    }];
                    return Action::Respond(format!("Updated to {}. What size?", describe_items(&state.items)));
                }
            }
            OrderPhase::Confirmed => {
                return Action::Respond("Your order is already confirmed. Is there anything else?".into());
            }
            OrderPhase::Cancelled => {
                state.phase = OrderPhase::AwaitingItem;
                state.items.clear();
                return Action::Respond("Starting fresh. What would you like to order?".into());
            }
        }

        Action::Continue
    }
}

fn parse_quantity(text: &str) -> Option<u32> {
    let number_words: Vec<(&str, u32)> = vec![
        ("one", 1), ("two", 2), ("three", 3), ("four", 4), ("five", 5),
        ("six", 6), ("seven", 7), ("eight", 8), ("nine", 9), ("ten", 10),
    ];
    let words: Vec<&str> = text.split_whitespace().collect();
    for (word, val) in &number_words {
        if words.iter().any(|w| {
            let trimmed = w.trim_matches(|c: char| !c.is_alphanumeric());
            trimmed == *word
        }) {
            return Some(*val);
        }
    }
    None
}

fn describe_items(items: &[OrderItem]) -> String {
    let parts: Vec<String> = items.iter().map(|i| {
        let size_str = i.size.as_deref().unwrap_or("regular");
        let plural = if i.quantity > 1 { "s" } else { "" };
        if i.quantity > 1 {
            format!("{} {} {}{}", i.quantity, size_str, i.name, plural)
        } else {
            format!("{} {}{}", size_str, i.name, plural)
        }
    }).collect();
    parts.join(", ")
}

/// Default café menu for order-flow agent.
pub fn default_menu() -> Vec<String> {
    vec![
        "coffee".into(),
        "latte".into(),
        "cappuccino".into(),
        "hot chocolate".into(),
        "tea".into(),
        "muffin".into(),
        "croissant".into(),
    ]
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
            if let Some(order) = obj.get("Order") {
                return Action::Order(order.clone());
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
                "president".into(), "manager".into(), "supervisor".into(),
                "escalate".into(), "complaint".into(),
            ],
            action: Action::Escalate("caller requested supervisor".into()),
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
    fn route_unmatched_gets_fallback_reply() {
        let router = IntentRouter::new(default_rules());
        let result = router.route("the weather is nice today");
        assert!(
            matches!(&result, Action::Respond(r) if r.contains("didn't quite catch")),
            "unmatched utterance should get the catch-all reply, got: {:?}",
            result
        );
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

    // -- word-boundary matching tests ---------------------------------------

    #[test]
    fn word_boundary_not_today_is_not_no_rule() {
        let router = IntentRouter::new(default_rules());
        let result = router.route("not today, thanks");
        // "not" is a single-word trigger — must not match inside "not today"
        assert!(
            !matches!(&result, Action::Respond(r) if r.contains("something else")),
            "'not today, thanks' should not trigger the 'not' rule, got: {:?}",
            result
        );
    }

    #[test]
    fn phrase_not_working_still_matches() {
        let router = IntentRouter::new(default_rules());
        let result = router.route("it is not working");
        // "not working" is a multi-word phrase trigger — must still match
        assert!(
            matches!(result, Action::Transfer(ref d) if d == "support"),
            "'it is not working' should transfer to support, got: {:?}",
            result
        );
    }

    #[test]
    fn word_boundary_nope_still_matches() {
        let router = IntentRouter::new(default_rules());
        let result = router.route("nope");
        // "nope" is a single-word trigger — should match the whole utterance
        assert!(
            matches!(&result, Action::Respond(r) if r.contains("something else")),
            "'nope' should trigger the no rule, got: {:?}",
            result
        );
    }

    #[test]
    fn word_boundary_yes_in_yes_please() {
        let router = IntentRouter::new(default_rules());
        let result = router.route("yes please");
        // "yes" is a single-word trigger — should match the whole word "yes"
        assert!(
            matches!(&result, Action::Respond(r) if r.contains("Great")),
            "'yes please' should trigger the yes rule, got: {:?}",
            result
        );
    }

    #[test]
    fn word_boundary_agent_not_in_agentic() {
        let router = IntentRouter::new(default_rules());
        let result = router.route("this is agentic behavior");
        // "agent" should only match as a whole word, not inside "agentic";
        // nothing else matches, so the catch-all fallback replies
        assert!(
            matches!(&result, Action::Respond(r) if r.contains("didn't quite catch")),
            "'agentic' should not trigger 'agent' rule, got: {:?}",
            result
        );
    }

    #[test]
    fn word_boundary_broken_still_matches() {
        let router = IntentRouter::new(default_rules());
        let result = router.route("my computer is broken");
        // "broken" is a single-word trigger in the support rule
        assert!(
            matches!(result, Action::Transfer(ref d) if d == "support"),
            "'my computer is broken' should transfer to support, got: {:?}",
            result
        );
    }

    #[test]
    fn word_boundary_agent_with_exclamation() {
        let router = IntentRouter::new(default_rules());
        let result = router.route("I want to speak to an agent!");
        // "agent!" should match trigger "agent" — punctuation is not a word boundary
        assert!(
            matches!(result, Action::Transfer(ref d) if d == "agent"),
            "'I want to speak to an agent!' should transfer to agent, got: {:?}",
            result
        );
    }

    #[test]
    fn word_boundary_manager_with_trailing_comma() {
        let router = IntentRouter::new(default_rules());
        let result = router.route("get me the manager,");
        // "manager," should match trigger "manager"
        assert!(
            matches!(result, Action::Escalate(_)),
            "'get me the manager,' should escalate, got: {:?}",
            result
        );
    }

    #[test]
    fn word_boundary_help_with_question_mark() {
        let router = IntentRouter::new(default_rules());
        let result = router.route("can you help?");
        // "help?" should match trigger "help" → support
        assert!(
            matches!(result, Action::Transfer(ref d) if d == "support"),
            "'can you help?' should transfer to support, got: {:?}",
            result
        );
    }

    // -- Order action parsing -----------------------------------------------

    #[test]
    fn parse_order_action() {
        let action = ProcessAgent::parse_action(r#"{"Order": {"item": "coffee", "quantity": 2}}"#);
        assert!(matches!(action, Action::Order(ref v) if v["item"] == "coffee"));
    }

    // -- parse_quantity word-boundary tests ---------------------------------

    #[test]
    fn parse_quantity_detects_number_words() {
        assert_eq!(parse_quantity("two lattes"), Some(2));
        assert_eq!(parse_quantity("make it three please"), Some(3));
        assert_eq!(parse_quantity("I'd like a coffee"), None);
    }

    #[test]
    fn parse_quantity_rejects_embedded_words() {
        // "one" inside "someone", "phone", "alone" must not match
        assert_eq!(parse_quantity("talk to someone"), None);
        assert_eq!(parse_quantity("call by phone"), None);
        assert_eq!(parse_quantity("leave me alone"), None);
    }

    #[test]
    fn parse_quantity_with_punctuation() {
        assert_eq!(parse_quantity("three!"), Some(3));
        assert_eq!(parse_quantity("give me two."), Some(2));
    }

    // -- describe_items tests ------------------------------------------------

    #[test]
    fn describe_items_singular() {
        let items = vec![OrderItem { name: "coffee".into(), quantity: 1, size: Some("large".into()) }];
        assert_eq!(describe_items(&items), "large coffee");
    }

    #[test]
    fn describe_items_multiple_quantity() {
        let items = vec![OrderItem { name: "latte".into(), quantity: 3, size: Some("medium".into()) }];
        assert_eq!(describe_items(&items), "3 medium lattes");
    }

    #[test]
    fn describe_items_multiple_items() {
        let items = vec![
            OrderItem { name: "coffee".into(), quantity: 1, size: Some("large".into()) },
            OrderItem { name: "muffin".into(), quantity: 2, size: None },
        ];
        assert_eq!(describe_items(&items), "large coffee, 2 regular muffins");
    }

    // -- OrderFlowAgent state machine ---------------------------------------

    #[test]
    fn order_flow_new_order() {
        let agent = OrderFlowAgent::new(default_menu());
        // Start with an item
        let action = agent.route("I'd like a coffee");
        assert!(
            matches!(&action, Action::Respond(r) if r.contains("size")),
            "should ask for size, got: {:?}", action
        );
    }

    #[test]
    fn order_flow_with_size() {
        let agent = OrderFlowAgent::new(default_menu());
        agent.route("I'd like a large coffee");
        let action = agent.route("yes");
        assert!(
            matches!(action, Action::Order(_)),
            "should emit order, got: {:?}", action
        );
    }

    #[test]
    fn order_flow_size_clarify() {
        let agent = OrderFlowAgent::new(default_menu());
        agent.route("I'd like a latte");
        let action = agent.route("large");
        assert!(
            matches!(&action, Action::Respond(r) if r.contains("Confirm")),
            "should confirm after size, got: {:?}", action
        );
    }

    #[test]
    fn order_flow_confirm() {
        let agent = OrderFlowAgent::new(default_menu());
        agent.route("I'd like a large coffee");
        let action = agent.route("yes");
        assert!(matches!(action, Action::Order(_)), "should emit order");
    }

    #[test]
    fn order_flow_correction() {
        let agent = OrderFlowAgent::new(default_menu());
        agent.route("I'd like a coffee");
        let action = agent.route("actually make it a large latte");
        assert!(
            matches!(&action, Action::Respond(r) if r.contains("Confirm")),
            "should confirm after correction, got: {:?}", action
        );
    }

    #[test]
    fn order_flow_cancel() {
        let agent = OrderFlowAgent::new(default_menu());
        agent.route("I'd like a coffee");
        let action = agent.route("cancel");
        assert!(
            matches!(&action, Action::Respond(r) if r.contains("cancelled")),
            "should cancel, got: {:?}", action
        );
    }

    #[test]
    fn order_flow_unknown_item() {
        let agent = OrderFlowAgent::new(default_menu());
        let action = agent.route("I'd like a pizza");
        assert!(
            matches!(&action, Action::Respond(r) if r.contains("What would you like")),
            "should prompt for known item, got: {:?}", action
        );
    }
}

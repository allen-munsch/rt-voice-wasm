//! Configuration file loading for native binaries.
//!
//! Reads `~/.config/rt-voice/config.toml` (or `--config PATH` override).
//! CLI flags always take precedence over config file values.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct RtVoiceConfig {
    #[serde(default = "default_model")]
    pub model: String,

    #[serde(default = "default_speed")]
    pub speed: f64,

    /// "whisper" or "moonshine"
    #[serde(default = "default_engine")]
    pub engine: String,

    #[serde(default)]
    pub device: Option<String>,

    #[serde(default)]
    pub agent_hook: Option<String>,

    #[serde(default)]
    pub wav_file: Option<String>,

    #[serde(default = "default_window_secs")]
    pub window_secs: f64,

    #[serde(default = "default_step_secs")]
    pub step_secs: f64,
}

fn default_model() -> String {
    // Resolved at load time — the config file's model path, or fallback to
    // system-installed / local paths checked by live_captions.
    String::new()
}
fn default_speed() -> f64 { 1.0 }
fn default_engine() -> String { "whisper".into() }
fn default_window_secs() -> f64 { 3.0 }
fn default_step_secs() -> f64 { 1.0 }

impl Default for RtVoiceConfig {
    fn default() -> Self {
        Self {
            model: String::new(),
            speed: 1.0,
            engine: "whisper".into(),
            device: None,
            agent_hook: None,
            wav_file: None,
            window_secs: 3.0,
            step_secs: 1.0,
        }
    }
}

impl RtVoiceConfig {
    /// Load config from the standard path, with optional `--config` CLI override.
    ///
    /// Resolution order:
    /// 1. `--config PATH` on command line
    /// 2. `$XDG_CONFIG_HOME/rt-voice/config.toml`
    /// 3. `~/.config/rt-voice/config.toml`
    /// 4. Built-in defaults
    pub fn load(args: &[String]) -> Self {
        let explicit_path = parse_flag(args, "--config");

        let mut config = if let Some(ref path) = explicit_path {
            Self::from_file(path).unwrap_or_default()
        } else {
            Self::from_xdg().unwrap_or_default()
        };

        // CLI flags override config file values
        if let Some(m) = parse_flag(args, "--model") {
            config.model = m;
        }
        if let Some(s) = parse_flag(args, "--speed") {
            if let Ok(v) = s.parse() {
                config.speed = v;
            }
        }
        if args.contains(&"--use-moonshine".to_string()) {
            config.engine = "moonshine".into();
        }
        if let Some(d) = parse_flag(args, "--device") {
            config.device = Some(d);
        }
        if let Some(h) = parse_flag(args, "--agent-hook") {
            config.agent_hook = Some(h);
        }
        if let Some(w) = parse_flag(args, "--wav-file") {
            config.wav_file = Some(w);
        }

        config
    }

    fn from_file(path: &str) -> Result<Self, String> {
        let data = std::fs::read_to_string(path)
            .map_err(|e| format!("read config {path}: {e}"))?;
        toml::from_str(&data)
            .map_err(|e| format!("parse config {path}: {e}"))
    }

    fn from_xdg() -> Result<Self, String> {
        let path = config_path();
        if path.exists() {
            Self::from_file(path.to_str().unwrap_or(""))
        } else {
            Err("no config file found".into())
        }
    }

    /// Resolve the model path: if set in config, use that; otherwise try
    /// system-installed then local paths.
    pub fn resolve_model_path(&self) -> String {
        if !self.model.is_empty() && std::path::Path::new(&self.model).exists() {
            return self.model.clone();
        }
        let candidates = [
            "/usr/share/rt-voice-wasm/models/ggml-tiny.en-q5_1.bin",
            "./models/ggml-tiny.en-q5_1.bin",
        ];
        for c in &candidates {
            if std::path::Path::new(c).exists() {
                return c.to_string();
            }
        }
        self.model.clone()
    }
}

fn config_path() -> std::path::PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        let p = std::path::PathBuf::from(xdg).join("rt-voice/config.toml");
        if p.exists() {
            return p;
        }
    }
    let home = dirs_fallback();
    home.join(".config/rt-voice/config.toml")
}

fn dirs_fallback() -> std::path::PathBuf {
    std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
}

fn parse_flag(args: &[String], flag: &str) -> Option<String> {
    let idx = args.iter().position(|a| a == flag)?;
    args.get(idx + 1).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let config = RtVoiceConfig::default();
        assert_eq!(config.speed, 1.0);
        assert_eq!(config.engine, "whisper");
        assert_eq!(config.window_secs, 3.0);
        assert_eq!(config.step_secs, 1.0);
        assert!(config.device.is_none());
        assert!(config.agent_hook.is_none());
    }

    #[test]
    fn cli_overrides_config() {
        let args = vec![
            "live-captions".into(),
            "--speed".into(), "2.0".into(),
            "--use-moonshine".into(),
            "--device".into(), "mic1".into(),
        ];
        let config = RtVoiceConfig::load(&args);
        assert_eq!(config.speed, 2.0);
        assert_eq!(config.engine, "moonshine");
        assert_eq!(config.device.as_deref(), Some("mic1"));
    }

    #[test]
    fn parse_toml() {
        let toml_str = r#"
speed = 1.5
engine = "moonshine"
window_secs = 5.0
device = "default"
"#;
        let config: RtVoiceConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.speed, 1.5);
        assert_eq!(config.engine, "moonshine");
        assert_eq!(config.window_secs, 5.0);
        assert_eq!(config.device.as_deref(), Some("default"));
    }

    #[test]
    fn parse_toml_minimal() {
        let config: RtVoiceConfig = toml::from_str("").unwrap();
        assert_eq!(config.speed, 1.0);
        assert_eq!(config.engine, "whisper");
    }
}

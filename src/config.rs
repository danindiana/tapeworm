/// User configuration loaded from ~/.config/tapeworm/config.toml.
/// All fields are optional — missing keys fall back to compiled-in defaults.
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub ollama: OllamaConfig,
    #[serde(default)]
    pub display: DisplayConfig,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OllamaConfig {
    #[serde(default = "default_ollama_url")]
    pub url: String,
    #[serde(default = "default_ollama_model")]
    pub model: String,
    /// Embed each command inline during `record` (requires Ollama to be reachable).
    #[serde(default)]
    pub auto_embed: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DisplayConfig {
    #[serde(default = "default_log_limit")]
    pub log_limit: usize,
}

fn default_ollama_url() -> String { crate::embed::DEFAULT_URL.to_string() }
fn default_ollama_model() -> String { crate::embed::DEFAULT_MODEL.to_string() }
fn default_log_limit() -> usize { 50 }

impl Default for OllamaConfig {
    fn default() -> Self {
        Self {
            url: default_ollama_url(),
            model: default_ollama_model(),
            auto_embed: false,
        }
    }
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self { log_limit: default_log_limit() }
    }
}

pub fn config_path() -> PathBuf {
    let mut p = dirs::config_local_dir()
        .unwrap_or_else(|| {
            PathBuf::from(format!(
                "{}/.config",
                std::env::var("HOME").unwrap_or_default()
            ))
        });
    p.push("tapeworm");
    p.push("config.toml");
    p
}

/// Load config from disk. Missing file → default config (never an error).
pub fn load() -> Config {
    let path = config_path();
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Config::default();
    };
    match toml::from_str::<Config>(&text) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("tapeworm: warning: could not parse config at {}: {}", path.display(), e);
            Config::default()
        }
    }
}

/// Severity of a validation issue.
#[derive(Debug, Clone, PartialEq)]
pub enum Severity { Error, Warning }

/// A single validation issue with its severity and message.
#[derive(Debug, Clone)]
pub struct Issue {
    pub severity: Severity,
    pub message:  String,
}

impl Issue {
    fn err(msg: impl Into<String>)  -> Self { Self { severity: Severity::Error,   message: msg.into() } }
    fn warn(msg: impl Into<String>) -> Self { Self { severity: Severity::Warning, message: msg.into() } }
}

/// Validate a loaded config and return any issues found.
/// An empty Vec means the config is fully valid.
pub fn validate(cfg: &Config) -> Vec<Issue> {
    let mut issues: Vec<Issue> = Vec::new();

    // ollama.url must look like an HTTP URL
    let url = cfg.ollama.url.trim();
    if url.is_empty() {
        issues.push(Issue::err("ollama.url is empty"));
    } else if !url.starts_with("http://") && !url.starts_with("https://") {
        issues.push(Issue::err(format!(
            "ollama.url does not start with http:// or https://: {:?}", url
        )));
    } else if url.contains(char::is_whitespace) {
        issues.push(Issue::err(format!("ollama.url contains whitespace: {:?}", url)));
    }

    // ollama.model must be non-empty
    if cfg.ollama.model.trim().is_empty() {
        issues.push(Issue::err("ollama.model is empty"));
    }

    // display.log_limit: 0 would silently show nothing
    if cfg.display.log_limit == 0 {
        issues.push(Issue::err("display.log_limit is 0 — `tapeworm log` would show no output"));
    } else if cfg.display.log_limit > 10_000 {
        issues.push(Issue::warn(format!(
            "display.log_limit is {} — very large default may be slow on big histories",
            cfg.display.log_limit
        )));
    }

    issues
}

/// Write a default config file if none exists. Returns the path.
pub fn init_default() -> Result<PathBuf> {
    let path = config_path();
    if path.exists() {
        return Ok(path);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let default_text = r#"# tapeworm configuration
# All values shown are defaults — delete any line to use the default.

[ollama]
url   = "http://localhost:11434"
model = "nomic-embed-text"
# Set to true to embed each command inline during `record`.
# Requires Ollama to be reachable; failures are silently ignored.
auto_embed = false

[display]
log_limit = 50
"#;
    std::fs::write(&path, default_text)?;
    Ok(path)
}

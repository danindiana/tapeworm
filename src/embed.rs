/// Ollama HTTP client for generating text embeddings.
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

pub const DEFAULT_MODEL: &str = "nomic-embed-text";
pub const DEFAULT_URL: &str = "http://localhost:11434";

pub struct OllamaClient {
    base_url: String,
    pub model: String,
    client: reqwest::blocking::Client,
}

/// POST /api/embed request body (newer Ollama batch endpoint)
#[derive(Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    input: &'a str,
}

/// POST /api/embed response
#[derive(Deserialize)]
struct EmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

impl OllamaClient {
    pub fn new(base_url: &str, model: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            model: model.to_string(),
            client: reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .expect("failed to build HTTP client"),
        }
    }

    /// Generate an embedding for a single text string.
    /// Returns a Vec<f32> whose length depends on the model
    /// (nomic-embed-text: 768 dims).
    pub fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let url = format!("{}/api/embed", self.base_url);
        let body = EmbedRequest { model: &self.model, input: text };

        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .with_context(|| format!("connecting to Ollama at {}", self.base_url))?
            .error_for_status()
            .context("Ollama returned an error status")?
            .json::<EmbedResponse>()
            .context("deserializing Ollama embed response")?;

        resp.embeddings
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("Ollama returned empty embeddings array"))
    }

    /// Verify the model is available; return its name on success.
    pub fn check_model(&self) -> Result<String> {
        let url = format!("{}/api/tags", self.base_url);
        #[derive(Deserialize)]
        struct TagsResp {
            models: Vec<ModelInfo>,
        }
        #[derive(Deserialize)]
        struct ModelInfo {
            name: String,
        }
        let resp: TagsResp = self
            .client
            .get(&url)
            .send()
            .with_context(|| format!("connecting to Ollama at {}", self.base_url))?
            .error_for_status()?
            .json()?;

        // Match on prefix since names can be "nomic-embed-text:latest" etc.
        let found = resp.models.iter().any(|m| {
            m.name == self.model || m.name.starts_with(&format!("{}:", self.model))
        });
        if found {
            Ok(self.model.clone())
        } else {
            Err(anyhow!(
                "model '{}' not found in Ollama. Run: ollama pull {}",
                self.model,
                self.model
            ))
        }
    }
}

/// Build the text that gets embedded for a command record.
/// Including CWD gives the model project context so that
/// "cargo build" in ~/tapeworm vs ~/other-project are distinguishable.
pub fn embed_text(command: &str, cwd: &str) -> String {
    format!("shell command: {} | directory: {}", command, cwd)
}

// --- Byte serialization for SQLite BLOB storage ---

/// Serialize a f32 slice as little-endian bytes.
pub fn vec_to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Deserialize f32 vec from little-endian bytes.
pub fn blob_to_vec(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4)
        .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_blob() {
        let original = vec![0.1f32, -0.5, 1.0, 0.0, f32::MAX];
        let blob = vec_to_blob(&original);
        let recovered = blob_to_vec(&blob);
        for (a, b) in original.iter().zip(recovered.iter()) {
            assert!((a - b).abs() < 1e-7);
        }
    }
}

use serde::{Deserialize, Serialize};

use veilux_kernel::Hash;

#[derive(Debug, thiserror::Error)]
pub enum OllamaError {
    #[error("http error: {0}")]
    Http(String),
    #[error("decode error: {0}")]
    Decode(String),
    #[error("ollama returned empty response")]
    Empty,
}

#[derive(Clone, Debug)]
pub struct OllamaConfig {
    pub base_url: String,
    pub model: String,
    pub timeout_secs: u64,
}

impl Default for OllamaConfig {
    fn default() -> Self {
        OllamaConfig {
            base_url: "http://localhost:11434".to_string(),
            model: "llama3".to_string(),
            timeout_secs: 120,
        }
    }
}

#[derive(Serialize)]
struct GenerateRequest<'a> {
    model: &'a str,
    prompt: &'a str,
    stream: bool,
}

#[derive(Deserialize)]
struct GenerateResponse {
    #[serde(default)]
    response: String,
    #[serde(default)]
    done: bool,
}

#[derive(Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    prompt: &'a str,
}

#[derive(Deserialize)]
struct EmbedResponse {
    #[serde(default)]
    embedding: Vec<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InferenceAttestation {
    pub model: String,
    pub prompt_hash: Hash,
    pub output: String,
    pub output_hash: Hash,
}

pub struct OllamaClient {
    cfg: OllamaConfig,
}

impl OllamaClient {
    pub fn new(cfg: OllamaConfig) -> Self {
        OllamaClient { cfg }
    }

    pub fn from_env() -> Self {
        let mut cfg = OllamaConfig::default();
        if let Ok(url) = std::env::var("VEILUX_OLLAMA_URL") {
            cfg.base_url = url;
        }
        if let Ok(model) = std::env::var("VEILUX_OLLAMA_MODEL") {
            cfg.model = model;
        }
        OllamaClient::new(cfg)
    }

    pub fn generate(&self, prompt: &str) -> Result<InferenceAttestation, OllamaError> {
        let url = format!("{}/api/generate", self.cfg.base_url.trim_end_matches('/'));
        let body = GenerateRequest {
            model: &self.cfg.model,
            prompt,
            stream: false,
        };
        let payload =
            serde_json::to_string(&body).map_err(|e| OllamaError::Decode(e.to_string()))?;

        let text = ureq::post(&url)
            .timeout(std::time::Duration::from_secs(self.cfg.timeout_secs))
            .set("Content-Type", "application/json")
            .send_string(&payload)
            .map_err(|e| OllamaError::Http(e.to_string()))?
            .into_string()
            .map_err(|e| OllamaError::Decode(e.to_string()))?;

        let parsed: GenerateResponse =
            serde_json::from_str(&text).map_err(|e| OllamaError::Decode(e.to_string()))?;

        if parsed.response.is_empty() && !parsed.done {
            return Err(OllamaError::Empty);
        }

        Ok(InferenceAttestation {
            model: self.cfg.model.clone(),
            prompt_hash: Hash::digest(prompt.as_bytes()),
            output_hash: Hash::digest(parsed.response.as_bytes()),
            output: parsed.response,
        })
    }

    pub fn embed(&self, text: &str) -> Result<Vec<f64>, OllamaError> {
        let url = format!("{}/api/embeddings", self.cfg.base_url.trim_end_matches('/'));
        let body = EmbedRequest {
            model: &self.cfg.model,
            prompt: text,
        };
        let payload =
            serde_json::to_string(&body).map_err(|e| OllamaError::Decode(e.to_string()))?;

        let resp_text = ureq::post(&url)
            .timeout(std::time::Duration::from_secs(self.cfg.timeout_secs))
            .set("Content-Type", "application/json")
            .send_string(&payload)
            .map_err(|e| OllamaError::Http(e.to_string()))?
            .into_string()
            .map_err(|e| OllamaError::Decode(e.to_string()))?;

        let parsed: EmbedResponse =
            serde_json::from_str(&resp_text).map_err(|e| OllamaError::Decode(e.to_string()))?;

        if parsed.embedding.is_empty() {
            return Err(OllamaError::Empty);
        }
        Ok(parsed.embedding)
    }
}

use anyhow::{Context, Result};
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct OpenRouter {
    client: Client,
    api_key: String,
    pub model: String,
    pub fallbacks: Vec<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub stream: bool,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<Vec<String>>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cost: Option<f64>,
    pub prompt_tokens_details: Option<TokenDetails>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct TokenDetails {
    pub cached_tokens: Option<u64>,
}
impl OpenRouter {
    pub fn from_env(model: String, fallbacks: Vec<String>) -> Result<Self> {
        Ok(Self {
            client: Client::new(),
            api_key: std::env::var("OPENROUTER_API_KEY")
                .context("OPENROUTER_API_KEY is not set")?,
            model,
            fallbacks,
        })
    }
    pub async fn stream<F>(
        &self,
        messages: Vec<Message>,
        session_id: &str,
        mut on_delta: F,
    ) -> Result<Usage>
    where
        F: FnMut(&str) + Send,
    {
        let request = ChatRequest {
            model: self.model.clone(),
            messages,
            stream: true,
            session_id: session_id.into(),
            models: (!self.fallbacks.is_empty()).then(|| self.fallbacks.clone()),
        };
        let response = self
            .client
            .post("https://openrouter.ai/api/v1/chat/completions")
            .bearer_auth(&self.api_key)
            .header("HTTP-Referer", "https://kairos.local")
            .json(&request)
            .send()
            .await?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let body = body.chars().take(1200).collect::<String>();
            anyhow::bail!("OpenRouter HTTP {status}: {body}");
        }
        let mut usage = Usage::default();
        let mut buffer = String::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            buffer.push_str(&String::from_utf8_lossy(&chunk?));
            while let Some(pos) = buffer.find('\n') {
                let line = buffer.drain(..=pos).collect::<String>();
                let line = line.trim();
                if let Some(data) = line.strip_prefix("data: ").filter(|d| *d != "[DONE]")
                    && let Ok(value) = serde_json::from_str::<serde_json::Value>(data)
                {
                    if let Some(text) = value["choices"][0]["delta"]["content"].as_str() {
                        on_delta(text);
                    }
                    if let Some(u) = value.get("usage") {
                        usage = serde_json::from_value(u.clone()).unwrap_or_default();
                    }
                }
            }
        }
        Ok(usage)
    }
    pub async fn prompt(
        &self,
        messages: Vec<Message>,
        session_id: &str,
    ) -> Result<(String, Usage)> {
        let mut text = String::new();
        let usage = self
            .stream(messages, session_id, |delta| text.push_str(delta))
            .await?;
        Ok((text, usage))
    }
}

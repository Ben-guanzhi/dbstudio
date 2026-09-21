use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Configuration for an LLM provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    pub provider: String,
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            provider: "openai".to_string(),
            api_key: None,
            model: Some("gpt-4o-mini".to_string()),
            base_url: None,
        }
    }
}

/// A message in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Role {
    System,
    User,
    Assistant,
}

/// Response from an LLM completion.
#[derive(Debug, Clone)]
pub struct CompletionResponse {
    pub content: String,
    pub tokens_used: Option<u32>,
}

/// Trait for LLM providers. Implement this to add new AI backends.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Get the name of this provider.
    fn name(&self) -> &str;

    /// Check if this provider is configured and ready.
    fn is_configured(&self) -> bool;

    /// Send a chat completion request.
    async fn chat(&self, messages: &[ChatMessage]) -> Result<CompletionResponse>;

    /// Send a simple completion request with a single prompt.
    async fn complete(&self, prompt: &str) -> Result<CompletionResponse> {
        self.chat(&[ChatMessage {
            role: Role::User,
            content: prompt.to_string(),
        }])
        .await
    }

    /// Explain a SQL query - returns optimization suggestions.
    async fn explain_sql(&self, sql: &str, schema_context: &str) -> Result<CompletionResponse> {
        let prompt = format!(
            "You are a database optimization expert. Analyze this SQL query and provide:\n\
             1. A brief explanation of what the query does\n\
             2. Potential performance issues\n\
             3. Suggested optimizations\n\
             4. Index recommendations if applicable\n\n\
             Schema context:\n{}\n\n\
             SQL Query:\n```sql\n{}\n```",
            schema_context, sql
        );
        self.complete(&prompt).await
    }

    /// Optimize a SQL query - returns an optimized version.
    async fn optimize_sql(&self, sql: &str, schema_context: &str) -> Result<CompletionResponse> {
        let prompt = format!(
            "You are a database optimization expert. Rewrite this SQL query for better performance. \
             Return ONLY the optimized SQL query, no explanation.\n\n\
             Schema context:\n{}\n\n\
             Original SQL:\n```sql\n{}\n```",
            schema_context, sql
        );
        self.complete(&prompt).await
    }
}

/// A mock provider for testing or when no AI is configured.
pub struct MockProvider;

#[async_trait]
impl LlmProvider for MockProvider {
    fn name(&self) -> &str {
        "mock"
    }

    fn is_configured(&self) -> bool {
        true
    }

    async fn chat(&self, messages: &[ChatMessage]) -> Result<CompletionResponse> {
        let last_message = messages
            .last()
            .map(|m| m.content.as_str())
            .unwrap_or("No message");

        Ok(CompletionResponse {
            content: format!(
                "[Mock AI Response] This is a placeholder response. \
                 Configure an AI provider in settings to get real responses.\n\n\
                 Your message: {}",
                last_message
            ),
            tokens_used: None,
        })
    }
}

/// The `openai` / `ollama` backends use the same HTTP contract: an
/// OpenAI-compatible `POST {base}/chat/completions` endpoint.
#[derive(Debug, Clone)]
pub struct OpenAiCompatibleProvider {
    pub config: LlmConfig,
}

fn role_str(role: &Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
    }
}

/// Resolve the effective endpoint URL and model name for a config, applying
/// per-provider defaults when the user left a field blank.
fn endpoint_for(config: &LlmConfig) -> (String, String) {
    let is_ollama = config.provider == "ollama";
    let base = config.base_url.clone().unwrap_or_else(|| {
        if is_ollama {
            "http://localhost:11434/v1".to_string()
        } else {
            "https://api.openai.com/v1".to_string()
        }
    });
    let model = config.model.clone().unwrap_or_else(|| {
        if is_ollama {
            "llama3".to_string()
        } else {
            "gpt-4o-mini".to_string()
        }
    });
    (format!("{}/chat/completions", base.trim_end_matches('/')), model)
}

/// Blocking call run on a helper thread; [`OpenAiCompatibleProvider::chat`]
/// bridges to it so the UI/async executor is never blocked by HTTP I/O.
fn openai_compatible_chat(
    config: &LlmConfig,
    messages: &[ChatMessage],
) -> Result<CompletionResponse> {
    use serde_json::{json, Value};

    let (url, model) = endpoint_for(config);
    let msgs: Vec<Value> = messages
        .iter()
        .map(|m| json!({ "role": role_str(&m.role), "content": m.content }))
        .collect();
    let body = json!({ "model": model, "messages": msgs });

    let mut request = ureq::post(&url).set("Content-Type", "application/json");
    if let Some(key) = config.api_key.as_deref().filter(|k| !k.is_empty()) {
        request = request.set("Authorization", &format!("Bearer {key}"));
    }

    let text = request
        .send_string(&body.to_string())
        .map_err(|err| match err {
            ureq::Error::Status(code, resp) => anyhow::anyhow!(
                "LLM returned HTTP {code}: {}",
                resp.into_string().unwrap_or_default()
            ),
            other => anyhow::anyhow!("LLM request failed: {other}"),
        })?
        .into_string()
        .map_err(|err| anyhow::anyhow!("failed to read LLM response: {err}"))?;

    let parsed: Value = serde_json::from_str(&text)
        .map_err(|err| anyhow::anyhow!("LLM response was not JSON: {err}"))?;
    let content = parsed["choices"][0]["message"]["content"]
        .as_str()
        .map(str::to_string)
        .unwrap_or(text);
    let tokens_used = parsed["usage"]["total_tokens"].as_u64().map(|t| t as u32);
    Ok(CompletionResponse {
        content,
        tokens_used,
    })
}

#[async_trait]
impl LlmProvider for OpenAiCompatibleProvider {
    fn name(&self) -> &str {
        &self.config.provider
    }

    fn is_configured(&self) -> bool {
        true
    }

    async fn chat(&self, messages: &[ChatMessage]) -> Result<CompletionResponse> {
        let config = self.config.clone();
        let messages = messages.to_vec();
        let (tx, rx) = async_channel::bounded::<Result<CompletionResponse>>(1);
        std::thread::spawn(move || {
            let _ = tx.try_send(openai_compatible_chat(&config, &messages));
        });
        rx.recv()
            .await
            .map_err(|err| anyhow::anyhow!("AI worker thread closed: {err}"))?
    }
}

/// Build a provider from a stored [`LlmConfig`]. Unknown provider names fall
/// back to the [`MockProvider`] so the app keeps working un-configured.
pub fn provider_for(config: &LlmConfig) -> Box<dyn LlmProvider> {
    match config.provider.as_str() {
        "openai" | "openai-compatible" | "ollama" => {
            Box::new(OpenAiCompatibleProvider {
                config: config.clone(),
            })
        }
        _ => Box::new(MockProvider),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_for_recognizes_supported_backends() {
        let mut config = LlmConfig::default();
        config.provider = "openai".into();
        assert_eq!(provider_for(&config).name(), "openai");
        config.provider = "ollama".into();
        assert_eq!(provider_for(&config).name(), "ollama");
        config.provider = "unknown".into();
        assert_eq!(provider_for(&config).name(), "mock");
    }

    #[test]
    fn endpoint_defaults_per_provider() {
        let mut config = LlmConfig::default();
        config.provider = "openai".into();
        config.base_url = None;
        config.model = None;
        let (url, model) = endpoint_for(&config);
        assert_eq!(url, "https://api.openai.com/v1/chat/completions");
        assert_eq!(model, "gpt-4o-mini");

        config.provider = "ollama".into();
        let (url, model) = endpoint_for(&config);
        assert_eq!(url, "http://localhost:11434/v1/chat/completions");
        assert_eq!(model, "llama3");

        config.base_url = Some("http://localhost:8080/v1".into());
        config.model = Some("qwen:7b".into());
        assert_eq!(
            endpoint_for(&config).0,
            "http://localhost:8080/v1/chat/completions"
        );
    }

    #[test]
    fn role_strings_match_openai_contract() {
        assert_eq!(role_str(&Role::System), "system");
        assert_eq!(role_str(&Role::User), "user");
        assert_eq!(role_str(&Role::Assistant), "assistant");
    }
}

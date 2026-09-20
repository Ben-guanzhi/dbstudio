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

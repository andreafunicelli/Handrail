//! LLM provider adapters. One neutral request/event model, two wire dialects:
//! Anthropic Messages and OpenAI Chat Completions (plus every OpenAI-compatible preset).

mod anthropic;
mod error;
mod http;
mod openai;
pub mod pricing;
mod sse;
mod types;

pub use anthropic::AnthropicProvider;
pub use error::{Error, Result};
pub use openai::OpenAiChatProvider;
pub use pricing::estimate_cost;
pub use types::*;

use aht_core::model::{Provider, ProviderKind, ProviderPreset};
use async_trait::async_trait;
use futures::stream::BoxStream;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

#[async_trait]
pub trait ChatProvider: Send + Sync {
    /// Start a streamed completion. Errors before the first token (auth, 404, 5xx after one
    /// automatic retry...) are returned here; mid-stream failures arrive as `Err` items.
    /// Cancelling `cancel` ends the stream silently.
    async fn stream(
        &self,
        req: ChatRequest,
        cancel: CancellationToken,
    ) -> Result<BoxStream<'static, Result<ProviderEvent>>>;

    async fn list_models(&self) -> Result<Vec<ModelInfo>>;
}

pub const ANTHROPIC_BASE: &str = "https://api.anthropic.com/v1";
pub const OPENAI_BASE: &str = "https://api.openai.com/v1";
pub const DEEPSEEK_BASE: &str = "https://api.deepseek.com";
pub const OPENCODE_ZEN_BASE: &str = "https://opencode.ai/zen/v1";
pub const OPENCODE_GO_BASE: &str = "https://opencode.ai/zen/go/v1";
pub const OPENROUTER_BASE: &str = "https://openrouter.ai/api/v1";
pub const OLLAMA_BASE: &str = "http://localhost:11434/v1";
pub const LMSTUDIO_BASE: &str = "http://localhost:1234/v1";

/// Default API base URL for a preset (`None` for `Custom`, which must supply one).
pub fn default_base_url(preset: ProviderPreset) -> Option<&'static str> {
    Some(match preset {
        ProviderPreset::Anthropic => ANTHROPIC_BASE,
        ProviderPreset::OpenAi => OPENAI_BASE,
        ProviderPreset::DeepSeek => DEEPSEEK_BASE,
        ProviderPreset::OpenCodeZen => OPENCODE_ZEN_BASE,
        ProviderPreset::OpenCodeGo => OPENCODE_GO_BASE,
        ProviderPreset::OpenRouter => OPENROUTER_BASE,
        ProviderPreset::Ollama => OLLAMA_BASE,
        ProviderPreset::LmStudio => LMSTUDIO_BASE,
        ProviderPreset::Custom => return None,
    })
}

/// OpenCode Zen/Go serve different model families on different endpoints. v1 supports
/// `/messages` (Claude models, Anthropic format) and `/chat/completions` (everything else).
/// Models that only exist on `/responses` (gpt-5.x, grok) or the Gemini endpoint are not
/// supported yet and will return an upstream error.
struct RoutedProvider {
    anthropic: AnthropicProvider,
    chat: OpenAiChatProvider,
}

fn strip_gateway_prefix(model: &str) -> &str {
    model
        .strip_prefix("opencode-go/")
        .or_else(|| model.strip_prefix("opencode/"))
        .unwrap_or(model)
}

#[async_trait]
impl ChatProvider for RoutedProvider {
    async fn stream(
        &self,
        mut req: ChatRequest,
        cancel: CancellationToken,
    ) -> Result<BoxStream<'static, Result<ProviderEvent>>> {
        req.model = strip_gateway_prefix(&req.model).to_string();
        if req.model.to_ascii_lowercase().starts_with("claude") {
            self.anthropic.stream(req, cancel).await
        } else {
            self.chat.stream(req, cancel).await
        }
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        self.chat.list_models().await
    }
}

/// A fresh random session id for gateways that route by conversation. Not a secret, only an
/// identifier, so std's randomly seeded hasher over the clock is enough (no extra crate).
fn new_session_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
    );
    format!("ses_{:016x}", h.finish())
}

/// Build a provider adapter from stored configuration. An empty `base_url` falls back to the
/// preset default. Never fails: a missing key simply produces `Auth` errors from the server.
pub fn from_config(provider: &Provider, api_key: Option<String>) -> Arc<dyn ChatProvider> {
    let base = if provider.base_url.trim().is_empty() {
        default_base_url(provider.preset)
            .unwrap_or(OPENAI_BASE)
            .to_string()
    } else {
        provider.base_url.trim().to_string()
    };
    match provider.preset {
        ProviderPreset::OpenCodeZen | ProviderPreset::OpenCodeGo => {
            // One id per provider instance: the app builds one per agent conversation, so every
            // request of a conversation carries the same id and a new conversation gets a new one.
            let session = new_session_id();
            Arc::new(RoutedProvider {
                anthropic: AnthropicProvider::new(&base, api_key.clone())
                    .with_bearer(true)
                    .with_opencode_session(session.clone()),
                chat: OpenAiChatProvider::new(&base, api_key).with_opencode_session(session),
            })
        }
        ProviderPreset::Anthropic => Arc::new(AnthropicProvider::new(base, api_key)),
        ProviderPreset::OpenAi => {
            Arc::new(OpenAiChatProvider::new(base, api_key).with_max_completion_tokens(true))
        }
        ProviderPreset::DeepSeek => {
            Arc::new(OpenAiChatProvider::new(base, api_key).with_echo_reasoning(true))
        }
        _ => match provider.kind {
            ProviderKind::Anthropic => Arc::new(AnthropicProvider::new(base, api_key)),
            ProviderKind::OpenAi => {
                Arc::new(OpenAiChatProvider::new(base, api_key).with_max_completion_tokens(true))
            }
            ProviderKind::OpenAiCompat => Arc::new(OpenAiChatProvider::new(base, api_key)),
        },
    }
}

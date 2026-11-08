//! `AiService`: provider selection, grammar fix and image generation.

use std::sync::Arc;

use frankmd_config::{AiProviderId, Feature, Key, ResolvedConfig, Value};
use secrecy::SecretString;

use crate::chat::{ChatTarget, bearer_value, complete};
use crate::http::{Header, post_json};
use crate::{
    AiError, DEFAULT_IMAGE_GENERATION_MODEL, Endpoints, GRAMMAR_PROMPT, GeneratedImage, GrammarFix,
    ImageGenerationInfo, ProviderInfo, ReferenceImages, image,
};

/// `AiService` bound to one config snapshot (the original rebuilt `Config.new`
/// on every call; callers pass the current `ResolvedConfig`).
#[derive(Clone)]
pub struct AiService {
    config: Arc<ResolvedConfig>,
    http: frankmd_net::TrustedClient,
    endpoints: Endpoints,
}

impl std::fmt::Debug for AiService {
    // The config holds credentials: never print it.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AiService")
            .field("endpoints", &self.endpoints)
            .finish_non_exhaustive()
    }
}

/// Ruby `String#blank?`: empty or only Unicode whitespace.
pub(crate) fn is_blank(text: &str) -> bool {
    text.chars().all(char::is_whitespace)
}

impl AiService {
    /// Production endpoints.
    pub fn new(config: Arc<ResolvedConfig>, http: frankmd_net::TrustedClient) -> Self {
        Self::with_endpoints(config, http, Endpoints::default())
    }

    /// Injected endpoints (tests point them at wiremock).
    pub fn with_endpoints(
        config: Arc<ResolvedConfig>,
        http: frankmd_net::TrustedClient,
        endpoints: Endpoints,
    ) -> Self {
        Self {
            config,
            http,
            endpoints,
        }
    }

    /// `enabled?` (`feature_available?("ai")`).
    pub fn enabled(&self) -> bool {
        self.config.feature_available(Feature::Ai)
    }

    /// `available_providers`.
    pub fn available_providers(&self) -> Vec<AiProviderId> {
        self.config.ai_providers_available()
    }

    /// `current_provider`.
    pub fn current_provider(&self) -> Option<AiProviderId> {
        self.config.effective_ai_provider()
    }

    /// `current_model`.
    pub fn current_model(&self) -> Option<String> {
        self.config.effective_ai_model()
    }

    /// Send a tiny real request to the selected provider/model and return
    /// success only when the provider answers with text. Used by Settings ›
    /// Integrations after saving an API key/model.
    pub async fn validate(&self) -> Result<(), AiError> {
        if !self.enabled() {
            return Err(AiError::NotConfigured);
        }
        let (Some(provider), Some(model)) = (self.current_provider(), self.current_model()) else {
            return Err(AiError::NoProvider);
        };
        tracing::info!(provider = provider.as_str(), model = %model, "AI validation request");
        let Some(target) = self.chat_target(provider, &model) else {
            return Err(AiError::NoProvider);
        };
        complete(
            &self.http,
            target,
            &model,
            "Reply with the single word OK.",
            "ping",
        )
        .await
        .map(|_| ())
        .map_err(|err| {
            tracing::error!(provider = provider.as_str(), model = %model, error = %err, "AI validation failed");
            AiError::ProviderFailed
        })
    }

    /// `provider_info`.
    pub fn provider_info(&self) -> ProviderInfo {
        ProviderInfo {
            enabled: self.enabled(),
            provider: self.current_provider(),
            model: self.current_model(),
            available_providers: self.available_providers(),
        }
    }

    /// `fix_grammar(text)`: guards (not configured → blank text → no provider),
    /// then one chat request with [`GRAMMAR_PROMPT`] as the system instruction
    /// and `text` as the single user message. Logs carry provider/model only
    /// (never keys or content).
    pub async fn fix_grammar(&self, text: Option<&str>) -> Result<GrammarFix, AiError> {
        if !self.enabled() {
            return Err(AiError::NotConfigured);
        }
        let text = text.filter(|t| !is_blank(t)).ok_or(AiError::NoText)?;
        let (Some(provider), Some(model)) = (self.current_provider(), self.current_model()) else {
            return Err(AiError::NoProvider);
        };
        tracing::info!(provider = provider.as_str(), "AI request");

        let result = match self.chat_target(provider, &model) {
            Some(target) => complete(&self.http, target, &model, GRAMMAR_PROMPT, text)
                .await
                .map_err(|e| e.to_string()),
            None => Err("provider endpoint not configured".to_owned()),
        };
        match result {
            Ok(corrected) => Ok(GrammarFix {
                corrected,
                provider,
                model,
            }),
            Err(reason) => {
                tracing::error!(provider = provider.as_str(), model = %model, error = %reason, "AI error");
                Err(AiError::ProviderFailed)
            }
        }
    }

    /// The request target for `provider`, holding **only** that provider's
    /// credential (the original cleared every other key first). `None` when
    /// the credential is missing or the Ollama base is not an http(s) URL.
    fn chat_target(&self, provider: AiProviderId, model: &str) -> Option<ChatTarget> {
        let credential = self.ai_string(provider.credential_key());
        let key = || credential.clone().map(SecretString::from);
        match provider {
            AiProviderId::Openai => Some(ChatTarget::OpenAiCompatible {
                url: format!("{}/v1/chat/completions", trim(&self.endpoints.openai)),
                bearer: Some(key()?),
                openrouter: false,
            }),
            AiProviderId::Openrouter => Some(ChatTarget::OpenAiCompatible {
                url: openrouter_url(&self.endpoints),
                bearer: Some(key()?),
                openrouter: true,
            }),
            AiProviderId::Deepseek => Some(ChatTarget::OpenAiCompatible {
                url: format!("{}/chat/completions", trim(&self.endpoints.deepseek)),
                bearer: Some(key()?),
                openrouter: false,
            }),
            AiProviderId::Ollama => Some(ChatTarget::OpenAiCompatible {
                url: ollama_url(credential.as_deref()?)?,
                bearer: None,
                openrouter: false,
            }),
            AiProviderId::Anthropic => Some(ChatTarget::Anthropic {
                url: format!("{}/v1/messages", trim(&self.endpoints.anthropic)),
                key: key()?,
            }),
            AiProviderId::Gemini => Some(ChatTarget::Gemini {
                url: gemini_url(&self.endpoints.gemini, model)?,
                key: key()?,
            }),
        }
    }

    /// `get_ai(key)` as a non-empty string.
    fn ai_string(&self, key: Key) -> Option<String> {
        match self.config.get_ai(key).value {
            Some(Value::Str(s)) if !is_blank(&s) => Some(s),
            _ => None,
        }
    }

    /// `image_generation_enabled?`.
    pub fn image_generation_enabled(&self) -> bool {
        use secrecy::ExposeSecret;
        self.config
            .image_generation_key()
            .is_some_and(|k| !is_blank(k.expose_secret()))
    }

    /// `image_generation_model`.
    pub fn image_generation_model(&self) -> String {
        match self.config.value(Key::ImageGenerationModel) {
            Some(Value::Str(s)) => s,
            _ => DEFAULT_IMAGE_GENERATION_MODEL.to_owned(),
        }
    }

    /// `image_generation_info`.
    pub fn image_generation_info(&self) -> ImageGenerationInfo {
        ImageGenerationInfo {
            enabled: self.image_generation_enabled(),
            model: self.image_generation_model(),
        }
    }

    /// `generate_image(prompt, reference_image_path:)`: guards (not configured →
    /// blank prompt), optional reference lookup via `refs` (missing → text-only),
    /// OpenRouter request, then [`crate::extract_image_from_response`].
    pub async fn generate_image(
        &self,
        prompt: Option<&str>,
        reference_image_path: Option<&str>,
        refs: &dyn ReferenceImages,
    ) -> Result<GeneratedImage, AiError> {
        let key = self
            .config
            .image_generation_key()
            .filter(|_| self.image_generation_enabled());
        let Some(key) = key else {
            return Err(AiError::ImageNotConfigured);
        };
        let prompt = prompt.filter(|p| !is_blank(p)).ok_or(AiError::NoPrompt)?;
        let model = self.image_generation_model();

        let reference = reference_image_path
            .filter(|p| !is_blank(p))
            .and_then(|path| {
                let found = refs.find(path);
                if found.is_none() {
                    tracing::warn!("reference image not found; generating from the prompt only");
                }
                found
            });
        tracing::info!(
            model = %model,
            prompt_length = prompt.chars().count(),
            reference = reference.is_some(),
            "image generation"
        );

        let headers = [Header::Secret("authorization", bearer_value(&key))];
        let body = image::request_body(&model, prompt, reference.as_ref());
        match post_json(
            &self.http,
            &openrouter_url(&self.endpoints),
            &headers,
            &body,
        )
        .await
        {
            Ok(answer) => {
                image::extract_image_from_response(&image::chat_response(&answer), &model)
            }
            Err(err) => {
                tracing::error!(model = %model, error = %err, "image generation error");
                Err(AiError::ImageFailed)
            }
        }
    }
}

fn trim(base: &str) -> &str {
    base.trim_end_matches('/')
}

fn openrouter_url(endpoints: &Endpoints) -> String {
    format!("{}/api/v1/chat/completions", trim(&endpoints.openrouter))
}

/// **[COMPAT]** `ollama_api_base` must parse as an http/https URL; requests go
/// to `<base>/chat/completions` (OpenAI-compatible, so the base ends in `/v1`).
fn ollama_url(base: &str) -> Option<String> {
    let parsed = url::Url::parse(base.trim()).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    Some(format!("{}/chat/completions", trim(base.trim())))
}

/// `<base>/v1beta/models/<model>:generateContent`, with the model encoded as
/// one path segment (a configured model can't add path or query parts).
fn gemini_url(base: &str, model: &str) -> Option<String> {
    let mut url = url::Url::parse(base).ok()?;
    url.path_segments_mut().ok()?.pop_if_empty().extend([
        "v1beta",
        "models",
        &format!("{model}:generateContent"),
    ]);
    Some(url.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ollama_base_must_be_http() {
        assert_eq!(
            ollama_url("http://localhost:11434/v1/").as_deref(),
            Some("http://localhost:11434/v1/chat/completions")
        );
        assert_eq!(ollama_url("file:///etc/passwd"), None);
        assert_eq!(ollama_url("localhost:11434"), None);
    }

    #[test]
    fn gemini_model_cannot_escape_its_segment() {
        let url = gemini_url("https://g.example", "../x?key=1").expect("url");
        assert_eq!(
            url,
            "https://g.example/v1beta/models/..%2Fx%3Fkey=1:generateContent"
        );
    }

    #[test]
    fn blank_matches_ruby() {
        assert!(is_blank(""));
        assert!(is_blank(" \n\t\u{3000}"));
        assert!(!is_blank(" a "));
    }
}

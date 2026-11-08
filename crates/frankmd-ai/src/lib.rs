//! # frankmd-ai
//!
//! Port of `app/services/ai_service.rb` + `app/controllers/ai_controller.rb`.
//! Provider selection follows `ResolvedConfig::effective_ai_provider` /
//! `effective_ai_model` (**[COMPAT]**, owned by frankmd-config). Chat requests
//! go through `frankmd-net`'s trusted client; image generation is an
//! OpenRouter chat completion with `modalities: ["text","image"]`.
//! Upstream error bodies / exception text are never surfaced (HP-011): every
//! failure collapses to the original's generic message.
//!
//! Wire shapes (base URLs injectable through [`Endpoints`] for tests):
//!
//! | provider | request | auth |
//! |---|---|---|
//! | openai | `POST {openai}/v1/chat/completions` | `Authorization: Bearer` |
//! | anthropic | `POST {anthropic}/v1/messages` (`system` = prompt) | `x-api-key`, `anthropic-version: 2023-06-01` |
//! | gemini | `POST {gemini}/v1beta/models/<model>:generateContent` | `x-goog-api-key` |
//! | openrouter | `POST {openrouter}/api/v1/chat/completions` | `Authorization: Bearer` |
//! | ollama | `POST <ollama_api_base>/chat/completions` (OpenAI-compatible) | none |
//!
//! (ruby_llm 2.1.1 used OpenAI's Responses API; chat completions is an
//! intentional, invisible-to-the-user divergence — see docs/testing/port-net-ai.md.)
//!
//! Modules: `service` (AiService), `chat` (wire formats), `image` (OpenRouter
//! image generation), `controller` (request validation), `http` (JSON POST +
//! ruby_llm retry policy). HTTP goes straight through `frankmd-net`'s trusted
//! client rather than `genai`: the four JSON shapes are small, the tests pin
//! them exactly, and genai has no OpenRouter image-modality support.
//!
//! **Security:** requests only ever carry the prompt/note text and the selected
//! provider's credential (as a sensitive header). The grammar check accepts
//! only markdown note paths ([`is_ai_note_path`]) — never `.fed` or dotfiles.

mod chat;
mod controller;
pub mod grammar;
mod http;
mod image;
mod service;

pub use controller::{
    AiRequestError, GrammarResponse, NoteSource, fix_grammar_note, generate_image_request,
    is_ai_note_path,
};
pub use image::{extract_image_from_response, parse_openrouter_response};
pub use service::AiService;

use frankmd_config::AiProviderId;

/// **[COMPAT]** `AiService::GRAMMAR_PROMPT`, verbatim (heredoc, trailing newline).
pub const GRAMMAR_PROMPT: &str = "You are a grammar and spelling corrector. Fix ONLY:
- Grammar errors
- Spelling mistakes
- Typos
- Punctuation errors

DO NOT change:
- Facts, opinions, or meaning
- Writing style or tone
- Markdown formatting (headers, links, code blocks, lists, etc.)
- Technical terms or proper nouns
- Code blocks or inline code

Return ONLY the corrected text with no explanations or commentary.
";

/// **[COMPAT]** default `image_generation_model`.
pub const DEFAULT_IMAGE_GENERATION_MODEL: &str = "google/gemini-3.1-flash-image-preview";

/// Result of a grammar fix: the corrected text (the UI diffs it with `similar`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrammarFix {
    /// Corrected document (`response.content`).
    pub corrected: String,
    /// Provider actually used.
    pub provider: AiProviderId,
    /// Model actually used (shown in the dialog).
    pub model: String,
}

/// `AiService.provider_info` / `GET /ai/config`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderInfo {
    /// Any provider configured.
    pub enabled: bool,
    /// Effective provider.
    pub provider: Option<AiProviderId>,
    /// Effective model.
    pub model: Option<String>,
    /// Configured providers (config order).
    pub available_providers: Vec<AiProviderId>,
}

/// `AiService.image_generation_info` / `GET /ai/image_config`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageGenerationInfo {
    /// OpenRouter key for images present (`.fed` value, else ENV — bypasses file-only mode).
    pub enabled: bool,
    /// `image_generation_model` or [`DEFAULT_IMAGE_GENERATION_MODEL`].
    pub model: String,
}

/// A generated image (`{data, mime_type, model, revised_prompt: nil}`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedImage {
    /// `Base64.strict_encode64` of the image bytes.
    pub data: String,
    /// Attachment MIME type, default `"image/png"`.
    pub mime_type: String,
    /// Model used.
    pub model: String,
}

/// A reference image handed to image generation (already resolved inside
/// `images_path` by the caller; frankmd-ai never touches the filesystem).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceImage {
    /// Raw bytes.
    pub bytes: Vec<u8>,
    /// MIME type (`image/png`, ...).
    pub mime_type: String,
}

/// **[COMPAT]** `ImagesService.find_image` seam: resolve `reference_image_path`.
/// `None` = missing → silently fall back to a text-only request.
pub trait ReferenceImages: Send + Sync {
    /// Look up a reference image by its `images_path`-relative path.
    fn find(&self, rel_path: &str) -> Option<ReferenceImage>;
}

/// No reference images available.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoReferenceImages;

impl ReferenceImages for NoReferenceImages {
    fn find(&self, _rel_path: &str) -> Option<ReferenceImage> {
        None
    }
}

/// One generated file in a provider response (`response.attachments`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    /// Decoded bytes.
    pub content: Vec<u8>,
    /// MIME type if the provider stated one.
    pub mime_type: Option<String>,
}

/// A parsed chat response (`RubyLLM::Message`: text + attachments).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChatResponse {
    /// Text content.
    pub content: String,
    /// Generated files (OpenRouter `message.images[]`, data URIs decoded).
    pub attachments: Vec<Attachment>,
}

/// AI errors. `Display` is the original's user-visible message, verbatim.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AiError {
    /// `{error: "AI not configured"}`.
    #[error("AI not configured")]
    NotConfigured,
    /// `{error: "No text provided"}`.
    #[error("No text provided")]
    NoText,
    /// `{error: "No AI provider available"}`.
    #[error("No AI provider available")]
    NoProvider,
    /// Any provider/transport/config failure (incl. a non-http Ollama base).
    #[error("AI provider request failed")]
    ProviderFailed,
    /// Image generation without an OpenRouter key.
    #[error("Image generation not configured. Requires OpenRouter API key.")]
    ImageNotConfigured,
    /// `{error: "No prompt provided"}`.
    #[error("No prompt provided")]
    NoPrompt,
    /// Any image-generation failure.
    #[error("Image generation failed")]
    ImageFailed,
    /// Provider answered without an image.
    #[error("No image data in response")]
    NoImageData,
}

/// Base URLs of the hosted providers (Ollama's comes from `ollama_api_base`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoints {
    /// `https://api.openai.com`.
    pub openai: String,
    /// `https://api.anthropic.com`.
    pub anthropic: String,
    /// `https://generativelanguage.googleapis.com`.
    pub gemini: String,
    /// `https://openrouter.ai`.
    pub openrouter: String,
    /// `https://api.deepseek.com`.
    pub deepseek: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            openai: "https://api.openai.com".into(),
            anthropic: "https://api.anthropic.com".into(),
            gemini: "https://generativelanguage.googleapis.com".into(),
            openrouter: "https://openrouter.ai".into(),
            deepseek: "https://api.deepseek.com".into(),
        }
    }
}

//! Shared helpers for frankmd-ai tests.
#![allow(dead_code, unreachable_pub)]

use std::sync::Arc;

use frankmd_ai::{AiService, Endpoints, ReferenceImage, ReferenceImages};
use frankmd_config::{EnvSnapshot, ResolvedConfig};

/// The original setup cleared every AI env var; tests pass only what they set.
pub fn config(fed: Option<&str>, env: &[(&str, &str)]) -> Arc<ResolvedConfig> {
    Arc::new(ResolvedConfig::new(
        fed,
        EnvSnapshot::from_pairs(env.iter().copied()),
    ))
}

pub fn service(fed: Option<&str>, env: &[(&str, &str)]) -> AiService {
    AiService::new(
        config(fed, env),
        frankmd_net::TrustedClient::new().expect("client"),
    )
}

/// Every hosted provider pointed at one mock server.
pub fn endpoints(base: &str) -> Endpoints {
    Endpoints {
        openai: base.into(),
        anthropic: base.into(),
        gemini: base.into(),
        openrouter: base.into(),
        deepseek: base.into(),
    }
}

pub fn mocked(base: &str, fed: Option<&str>, env: &[(&str, &str)]) -> AiService {
    AiService::with_endpoints(
        config(fed, env),
        frankmd_net::TrustedClient::new().expect("client"),
        endpoints(base),
    )
}

/// In-memory `ImagesService.find_image`.
#[derive(Debug, Default)]
pub struct Refs(pub Vec<(String, ReferenceImage)>);

impl ReferenceImages for Refs {
    fn find(&self, rel_path: &str) -> Option<ReferenceImage> {
        self.0
            .iter()
            .find(|(p, _)| p == rel_path)
            .map(|(_, r)| r.clone())
    }
}

/// OpenAI-style chat completion body.
pub fn chat_completion(content: &str) -> serde_json::Value {
    serde_json::json!({"choices": [{"index": 0, "message": {"role": "assistant", "content": content}, "finish_reason": "stop"}]})
}

/// OpenRouter image response: `message.images[].image_url.url` data URI.
pub fn openrouter_image(text: &str, mime: &str, b64: &str) -> serde_json::Value {
    serde_json::json!({"choices": [{"message": {"role": "assistant", "content": text,
        "images": [{"type": "image_url", "image_url": {"url": format!("data:{mime};base64,{b64}")}}]}}]})
}

/// `Base64.strict_encode64`.
pub fn b64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// 1×1 PNG from the original test.
pub const PNG_1X1: [u8; 70] = [
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53,
    0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08, 0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00,
    0x00, 0x00, 0x03, 0x00, 0x01, 0x00, 0x05, 0xFE, 0xD4, 0xE7, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
    0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

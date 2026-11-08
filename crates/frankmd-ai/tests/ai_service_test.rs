//! Port of `test/services/ai_service_test.rb` (`AiServiceTest` 42 + `AiServiceImageGenerationTest` 5).
//! The original cleared every AI env var in setup; here each test builds its own
//! `EnvSnapshot`. RubyLLM mocks become wiremock servers speaking each provider's
//! HTTP shape (see the table in `frankmd_ai` docs).
#![allow(clippy::disallowed_methods)]

mod common;

use common::*;
use frankmd_ai::{
    AiError, Attachment, ChatResponse, GRAMMAR_PROMPT, NoReferenceImages, ReferenceImage,
    extract_image_from_response,
};
use frankmd_config::AiProviderId;
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// port: test/services/ai_service_test.rb:34 "enabled? returns false when no providers configured"
#[test]
fn enabled_false_when_no_providers_configured() {
    assert!(!service(None, &[]).enabled());
}

// port: test/services/ai_service_test.rb:38 "enabled? returns true when OpenAI key is set"
#[test]
fn enabled_with_openai_key() {
    assert!(service(None, &[("OPENAI_API_KEY", "sk-test-key")]).enabled());
}

// port: test/services/ai_service_test.rb:43 "enabled? returns true when OpenRouter key is set"
#[test]
fn enabled_with_openrouter_key() {
    assert!(service(None, &[("OPENROUTER_API_KEY", "sk-or-test-key")]).enabled());
}

// port: test/services/ai_service_test.rb:48 "enabled? returns true when Anthropic key is set"
#[test]
fn enabled_with_anthropic_key() {
    assert!(service(None, &[("ANTHROPIC_API_KEY", "sk-ant-test-key")]).enabled());
}

// port: test/services/ai_service_test.rb:53 "enabled? returns true when Gemini key is set"
#[test]
fn enabled_with_gemini_key() {
    assert!(service(None, &[("GEMINI_API_KEY", "gemini-test-key")]).enabled());
}

// port: test/services/ai_service_test.rb:58 "enabled? returns true when Ollama base is set"
#[test]
fn enabled_with_ollama_base() {
    assert!(service(None, &[("OLLAMA_API_BASE", "http://localhost:11434")]).enabled());
}

// port: test/services/ai_service_test.rb:66 "current_provider returns openai when multiple providers available"
#[test]
fn current_provider_openai_when_multiple() {
    assert_eq!(
        service(
            None,
            &[
                ("OLLAMA_API_BASE", "http://localhost:11434"),
                ("OPENAI_API_KEY", "sk-test"),
                ("ANTHROPIC_API_KEY", "sk-ant-test")
            ]
        )
        .current_provider(),
        Some(AiProviderId::Openai)
    );
}

// port: test/services/ai_service_test.rb:74 "current_provider returns anthropic when openai not available"
#[test]
fn current_provider_anthropic_without_openai() {
    assert_eq!(
        service(
            None,
            &[
                ("ANTHROPIC_API_KEY", "sk-ant-test"),
                ("GEMINI_API_KEY", "gemini-test")
            ]
        )
        .current_provider(),
        Some(AiProviderId::Anthropic)
    );
}

// port: test/services/ai_service_test.rb:81 "current_provider returns openrouter when higher priority not available"
#[test]
fn current_provider_openrouter_when_higher_missing() {
    assert_eq!(
        service(
            None,
            &[
                ("OPENROUTER_API_KEY", "sk-or-test"),
                ("OLLAMA_API_BASE", "http://localhost:11434"),
                ("GEMINI_API_KEY", "gemini-test")
            ]
        )
        .current_provider(),
        Some(AiProviderId::Openrouter)
    );
}

// port: test/services/ai_service_test.rb:89 "current_provider returns ollama over gemini"
#[test]
fn current_provider_ollama_over_gemini() {
    assert_eq!(
        service(
            None,
            &[
                ("OLLAMA_API_BASE", "http://localhost:11434"),
                ("GEMINI_API_KEY", "gemini-test")
            ]
        )
        .current_provider(),
        Some(AiProviderId::Ollama)
    );
}

// port: test/services/ai_service_test.rb:96 "current_provider returns gemini when only gemini available"
#[test]
fn current_provider_gemini_alone() {
    assert_eq!(
        service(None, &[("GEMINI_API_KEY", "gemini-test")]).current_provider(),
        Some(AiProviderId::Gemini)
    );
}

// port: test/services/ai_service_test.rb:103 "current_provider respects ai_provider override"
#[test]
fn current_provider_respects_override() {
    assert_eq!(
        service(
            None,
            &[
                ("OLLAMA_API_BASE", "http://localhost:11434"),
                ("OPENAI_API_KEY", "sk-test"),
                ("AI_PROVIDER", "openai")
            ]
        )
        .current_provider(),
        Some(AiProviderId::Openai)
    );
}

// port: test/services/ai_service_test.rb:111 "current_provider falls back to priority when override not available"
#[test]
fn current_provider_override_unavailable_falls_back() {
    assert_eq!(
        service(
            None,
            &[
                ("OLLAMA_API_BASE", "http://localhost:11434"),
                ("AI_PROVIDER", "openai")
            ]
        )
        .current_provider(),
        Some(AiProviderId::Ollama)
    );
}

// port: test/services/ai_service_test.rb:119 "current_model returns provider-specific default"
#[test]
fn current_model_openai_default() {
    assert_eq!(
        service(None, &[("OPENAI_API_KEY", "sk-test")])
            .current_model()
            .as_deref(),
        Some("gpt-4o-mini")
    );
}

// port: test/services/ai_service_test.rb:124 "current_model returns ollama default model"
#[test]
fn current_model_ollama_default() {
    assert_eq!(
        service(None, &[("OLLAMA_API_BASE", "http://localhost:11434")])
            .current_model()
            .as_deref(),
        Some("llama3.2:latest")
    );
}

// port: test/services/ai_service_test.rb:129 "current_model returns anthropic default model"
#[test]
fn current_model_anthropic_default() {
    assert_eq!(
        service(None, &[("ANTHROPIC_API_KEY", "sk-ant-test")])
            .current_model()
            .as_deref(),
        Some("claude-sonnet-4-20250514")
    );
}

// port: test/services/ai_service_test.rb:134 "current_model returns gemini default model"
#[test]
fn current_model_gemini_default() {
    assert_eq!(
        service(None, &[("GEMINI_API_KEY", "gemini-test")])
            .current_model()
            .as_deref(),
        Some("gemini-2.0-flash")
    );
}

// port: test/services/ai_service_test.rb:139 "current_model returns openrouter default model"
#[test]
fn current_model_openrouter_default() {
    assert_eq!(
        service(None, &[("OPENROUTER_API_KEY", "sk-or-test")])
            .current_model()
            .as_deref(),
        Some("openai/gpt-4o-mini")
    );
}

// port: test/services/ai_service_test.rb:144 "current_model respects ai_model global override"
#[test]
fn current_model_global_override() {
    assert_eq!(
        service(
            None,
            &[("OPENAI_API_KEY", "sk-test"), ("AI_MODEL", "gpt-4-turbo")]
        )
        .current_model()
        .as_deref(),
        Some("gpt-4-turbo")
    );
}

// port: test/services/ai_service_test.rb:151 "current_model respects provider-specific model override"
#[test]
fn current_model_provider_override() {
    assert_eq!(
        service(
            None,
            &[
                ("ANTHROPIC_API_KEY", "sk-ant-test"),
                ("ANTHROPIC_MODEL", "claude-3-opus-20240229")
            ]
        )
        .current_model()
        .as_deref(),
        Some("claude-3-opus-20240229")
    );
}

// port: test/services/ai_service_test.rb:159 "fix_grammar returns error when not configured"
#[tokio::test]
async fn fix_grammar_not_configured() {
    let err = service(None, &[])
        .fix_grammar(Some("Hello world"))
        .await
        .expect_err("not configured");
    assert_eq!(err.to_string(), "AI not configured");
}

// port: test/services/ai_service_test.rb:164 "fix_grammar returns error when text is blank"
#[tokio::test]
async fn fix_grammar_blank_text() {
    let err = service(None, &[("OPENAI_API_KEY", "sk-test-key")])
        .fix_grammar(Some(""))
        .await
        .expect_err("blank");
    assert_eq!(err, AiError::NoText);
    assert_eq!(err.to_string(), "No text provided");
}

// port: test/services/ai_service_test.rb:170 "fix_grammar returns error when text is nil"
#[tokio::test]
async fn fix_grammar_nil_text() {
    let err = service(None, &[("OPENAI_API_KEY", "sk-test-key")])
        .fix_grammar(None)
        .await
        .expect_err("nil");
    assert_eq!(err.to_string(), "No text provided");
}

// port: test/services/ai_service_test.rb:177 "provider_info returns correct structure"
#[test]
fn provider_info_structure() {
    let info = service(
        None,
        &[
            ("OPENAI_API_KEY", "sk-test"),
            ("ANTHROPIC_API_KEY", "sk-ant-test"),
        ],
    )
    .provider_info();
    assert!(info.enabled);
    assert_eq!(info.provider, Some(AiProviderId::Openai)); // openai has highest priority
    assert!(info.model.is_some());
    assert!(info.available_providers.contains(&AiProviderId::Openai));
    assert!(info.available_providers.contains(&AiProviderId::Anthropic));
}

// port: test/services/ai_service_test.rb:194 "available_providers returns all configured providers"
#[test]
fn available_providers_lists_configured() {
    let providers = service(
        None,
        &[
            ("OLLAMA_API_BASE", "http://localhost:11434"),
            ("OPENAI_API_KEY", "sk-test"),
            ("ANTHROPIC_API_KEY", "sk-ant-test"),
        ],
    )
    .available_providers();
    assert!(providers.contains(&AiProviderId::Ollama));
    assert!(providers.contains(&AiProviderId::Openai));
    assert!(providers.contains(&AiProviderId::Anthropic));
    assert!(!providers.contains(&AiProviderId::Gemini));
    assert!(!providers.contains(&AiProviderId::Openrouter));
}

// port: test/services/ai_service_test.rb:209 "image_generation_enabled? returns false when no OpenRouter key"
#[test]
fn image_generation_disabled_without_key() {
    assert!(!service(None, &[]).image_generation_enabled());
}

// port: test/services/ai_service_test.rb:213 "image_generation_enabled? returns true when OpenRouter key is set"
#[test]
fn image_generation_enabled_with_key() {
    assert!(service(None, &[("OPENROUTER_API_KEY", "sk-or-test-key")]).image_generation_enabled());
}

// port: test/services/ai_service_test.rb:218 "image_generation_model returns default model"
#[test]
fn image_generation_model_default() {
    assert_eq!(
        service(None, &[]).image_generation_model(),
        "google/gemini-3.1-flash-image-preview"
    );
}

// port: test/services/ai_service_test.rb:222 "image_generation_info returns correct structure"
#[test]
fn image_generation_info_structure() {
    let info = service(None, &[]).image_generation_info();
    assert!(!info.enabled);
    assert_eq!(info.model, "google/gemini-3.1-flash-image-preview");
}

// port: test/services/ai_service_test.rb:230 "image_generation_info returns enabled when OpenRouter key set"
#[test]
fn image_generation_info_enabled() {
    assert!(
        service(None, &[("OPENROUTER_API_KEY", "sk-or-test-key")])
            .image_generation_info()
            .enabled
    );
}

// port: test/services/ai_service_test.rb:236 "generate_image returns error when not configured"
#[tokio::test]
async fn generate_image_not_configured() {
    let err = service(None, &[])
        .generate_image(Some("A sunset over mountains"), None, &NoReferenceImages)
        .await
        .expect_err("nc");
    assert!(err.to_string().contains("not configured"), "{err}");
}

// port: test/services/ai_service_test.rb:241 "generate_image returns error when prompt is blank"
#[tokio::test]
async fn generate_image_blank_prompt() {
    let s = service(None, &[("OPENROUTER_API_KEY", "sk-or-test-key")]);
    let err = s
        .generate_image(Some(""), None, &NoReferenceImages)
        .await
        .expect_err("blank");
    assert_eq!(err.to_string(), "No prompt provided");
}

// port: test/services/ai_service_test.rb:247 "generate_image returns error when prompt is nil"
#[tokio::test]
async fn generate_image_nil_prompt() {
    let s = service(None, &[("OPENROUTER_API_KEY", "sk-or-test-key")]);
    let err = s
        .generate_image(None, None, &NoReferenceImages)
        .await
        .expect_err("nil");
    assert_eq!(err.to_string(), "No prompt provided");
}

// port: test/services/ai_service_test.rb:253 "generate_image accepts reference_image_path parameter"
#[tokio::test]
async fn generate_image_accepts_reference_path() {
    // The original hit the real OpenRouter API with a fake key (WebMock allowed net);
    // here the mock answers 401 like OpenRouter would. Missing reference → text-only request.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_string(r#"{"error":{"message":"No auth credentials found"}}"#),
        )
        .mount(&server)
        .await;
    let s = mocked(
        &server.uri(),
        None,
        &[("OPENROUTER_API_KEY", "sk-or-test-key")],
    );
    let err = s
        .generate_image(Some("A sunset"), Some("nonexistent.jpg"), &Refs::default())
        .await
        .expect_err("401");
    assert!(!err.to_string().is_empty());
    assert!(
        !err.to_string().contains("credentials"),
        "upstream body leaked: {err}"
    );
}

// port: test/services/ai_service_test.rb:265 "fix_grammar returns corrected text on success"
#[tokio::test]
async fn fix_grammar_success_openai() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer sk-test-key"))
        .and(body_partial_json(
            serde_json::json!({"model": "gpt-4o-mini", "messages": [
            {"role": "system", "content": GRAMMAR_PROMPT},
            {"role": "user", "content": "This is uncorrected text"}]}),
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(chat_completion("This is corrected text.")),
        )
        .expect(1)
        .mount(&server)
        .await;
    let s = mocked(&server.uri(), None, &[("OPENAI_API_KEY", "sk-test-key")]);
    let fix = s
        .fix_grammar(Some("This is uncorrected text"))
        .await
        .expect("fix");
    assert_eq!(fix.corrected, "This is corrected text.");
    assert_eq!(fix.provider, AiProviderId::Openai);
    assert_eq!(fix.model, "gpt-4o-mini");
}

// port: test/services/ai_service_test.rb:283 "fix_grammar returns error on API failure"
#[tokio::test]
async fn fix_grammar_api_failure_is_generic() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(500).set_body_string("API connection failed"))
        .mount(&server)
        .await;
    let s = mocked(&server.uri(), None, &[("OPENAI_API_KEY", "sk-test-key")]);
    let err = s.fix_grammar(Some("Some text")).await.expect_err("500");
    assert_eq!(err.to_string(), "AI provider request failed");
    assert!(!err.to_string().contains("API connection failed"));
}

// port: test/services/ai_service_test.rb:299 "fix_grammar does not log an API key fragment"
#[tokio::test]
async fn fix_grammar_never_logs_key_fragment() {
    use std::sync::{Arc, Mutex};
    #[derive(Clone, Default)]
    struct Buf(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Buf {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("lock").extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let buf = Buf::default();
    let writer = buf.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(move || writer.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(chat_completion("Corrected")))
        .mount(&server)
        .await;
    let s = mocked(
        &server.uri(),
        None,
        &[("OPENAI_API_KEY", "sk-sensitive-key")],
    );
    let _ = s.fix_grammar(Some("Some text")).await;
    let logged = String::from_utf8_lossy(&buf.0.lock().expect("lock")).into_owned();
    assert!(
        !logged.contains("sk-sensiti"),
        "key fragment logged: {logged}"
    );
}

// port: test/services/ai_service_test.rb:314 "fix_grammar rejects an Ollama URL without an HTTP scheme"
#[tokio::test]
async fn fix_grammar_rejects_non_http_ollama_base() {
    let s = service(None, &[("OLLAMA_API_BASE", "file:///etc/passwd")]);
    let err = s
        .fix_grammar(Some("Some text"))
        .await
        .expect_err("bad base");
    assert_eq!(err.to_string(), "AI provider request failed");
}

// port: test/services/ai_service_test.rb:322 "fix_grammar works with anthropic provider"
#[tokio::test]
async fn fix_grammar_anthropic() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/messages"))
        .and(header("x-api-key", "sk-ant-test"))
        .and(header("anthropic-version", "2023-06-01"))
        .and(body_partial_json(serde_json::json!({"model": "claude-sonnet-4-20250514", "system": GRAMMAR_PROMPT,
            "messages": [{"role": "user", "content": "Input text"}]})))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "type": "message", "role": "assistant", "content": [{"type": "text", "text": "Fixed by Claude"}]})))
        .expect(1).mount(&server).await;
    let s = mocked(&server.uri(), None, &[("ANTHROPIC_API_KEY", "sk-ant-test")]);
    let fix = s.fix_grammar(Some("Input text")).await.expect("fix");
    assert_eq!(fix.corrected, "Fixed by Claude");
    assert_eq!(fix.provider, AiProviderId::Anthropic);
}

// port: test/services/ai_service_test.rb:340 "extract_image_from_response extracts base64 from a Message with attachments"
#[test]
fn extract_image_from_attachments() {
    let resp = ChatResponse {
        content: "Image generated".into(),
        attachments: vec![Attachment {
            content: b"binary_image_data".to_vec(),
            mime_type: Some("image/png".into()),
        }],
    };
    let img =
        extract_image_from_response(&resp, "google/gemini-3.1-flash-image-preview").expect("image");
    assert_eq!(img.data, b64(b"binary_image_data"));
    assert_eq!(img.mime_type, "image/png");
    assert_eq!(img.model, "google/gemini-3.1-flash-image-preview");
}

// port: test/services/ai_service_test.rb:352 "extract_image_from_response returns error when no attachments"
#[test]
fn extract_image_no_attachments() {
    let resp = ChatResponse {
        content: "I cannot generate that image".into(),
        attachments: vec![],
    };
    let err = extract_image_from_response(&resp, "model").expect_err("none");
    assert_eq!(err.to_string(), "No image data in response");
}

// port: test/services/ai_service_test.rb:360 "extract_image_from_response returns error for a text-only response"
#[test]
fn extract_image_text_only() {
    let resp = ChatResponse {
        content: "Some text response".into(),
        attachments: vec![],
    };
    assert_eq!(
        extract_image_from_response(&resp, "model"),
        Err(AiError::NoImageData)
    );
}

// --- AiServiceImageGenerationTest: Config stubbed with `.fed openrouter_api_key = sk-or-test-key` ---

const IMG_FED: &str = "openrouter_api_key = sk-or-test-key\n";

async fn openrouter_mock() -> MockServer {
    MockServer::start().await
}

// port: test/services/ai_service_test.rb:412 "generate_image text-only returns image data on success"
#[tokio::test]
async fn image_gen_text_only_success() {
    let server = openrouter_mock().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/chat/completions"))
        .and(header("authorization", "Bearer sk-or-test-key"))
        .and(body_partial_json(
            serde_json::json!({"model": "google/gemini-3.1-flash-image-preview",
            "modalities": ["text", "image"],
            "messages": [{"role": "user", "content": "A sunset over mountains"}]}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(openrouter_image(
            "Image generated",
            "image/png",
            &b64(b"png_binary_data"),
        )))
        .expect(1)
        .mount(&server)
        .await;
    let s = mocked(&server.uri(), Some(IMG_FED), &[]);
    let img = s
        .generate_image(Some("A sunset over mountains"), None, &NoReferenceImages)
        .await
        .expect("image");
    assert_eq!(img.data, b64(b"png_binary_data"));
    assert_eq!(img.mime_type, "image/png");
    assert_eq!(img.model, "google/gemini-3.1-flash-image-preview");
}

// port: test/services/ai_service_test.rb:429 "generate_image handles API error"
#[tokio::test]
async fn image_gen_api_error_is_generic() {
    let server = openrouter_mock().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(400).set_body_string("OpenRouter API error: Invalid prompt"),
        )
        .mount(&server)
        .await;
    let s = mocked(&server.uri(), Some(IMG_FED), &[]);
    let err = s
        .generate_image(Some("bad prompt"), None, &NoReferenceImages)
        .await
        .expect_err("400");
    assert_eq!(err.to_string(), "Image generation failed");
}

// port: test/services/ai_service_test.rb:441 "generate_image with reference passes the image via with:"
#[tokio::test]
async fn image_gen_with_reference_attaches_image() {
    let server = openrouter_mock().await;
    let data_uri = format!("data:image/png;base64,{}", b64(&PNG_1X1));
    Mock::given(method("POST"))
        .and(path("/api/v1/chat/completions"))
        .and(body_partial_json(
            serde_json::json!({"messages": [{"role": "user", "content": [
            {"type": "text", "text": "Make it brighter"},
            {"type": "image_url", "image_url": {"url": data_uri}}]}]}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(openrouter_image(
            "Edited image",
            "image/jpeg",
            &b64(b"edited_binary_data"),
        )))
        .expect(1)
        .mount(&server)
        .await;
    let refs = Refs(vec![(
        "ref_image.png".into(),
        ReferenceImage {
            bytes: PNG_1X1.to_vec(),
            mime_type: "image/png".into(),
        },
    )]);
    let s = mocked(&server.uri(), Some(IMG_FED), &[]);
    let img = s
        .generate_image(Some("Make it brighter"), Some("ref_image.png"), &refs)
        .await
        .expect("image");
    assert_eq!(img.data, b64(b"edited_binary_data"));
    assert_eq!(img.mime_type, "image/jpeg");
    assert_eq!(img.model, "google/gemini-3.1-flash-image-preview");
}

// port: test/services/ai_service_test.rb:470 "generate_image handles text-only response (no image generated)"
#[tokio::test]
async fn image_gen_text_only_response() {
    let server = openrouter_mock().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(chat_completion("I cannot generate that image")),
        )
        .mount(&server)
        .await;
    let s = mocked(&server.uri(), Some(IMG_FED), &[]);
    let err = s
        .generate_image(
            Some("Generate something impossible"),
            None,
            &NoReferenceImages,
        )
        .await
        .expect_err("no image");
    assert_eq!(err.to_string(), "No image data in response");
}

// port: test/services/ai_service_test.rb:483 "generate_image falls back to text-only when reference image not found"
#[tokio::test]
async fn image_gen_missing_reference_falls_back() {
    let server = openrouter_mock().await;
    // When the ref image is not found, the prompt is asked with no attachment (plain string content).
    Mock::given(method("POST"))
        .and(path("/api/v1/chat/completions"))
        .and(body_partial_json(
            serde_json::json!({"messages": [{"role": "user", "content": "A cat"}]}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(openrouter_image(
            "Generated",
            "image/png",
            &b64(b"fallback_data"),
        )))
        .expect(1)
        .mount(&server)
        .await;
    let s = mocked(&server.uri(), Some(IMG_FED), &[]);
    let img = s
        .generate_image(Some("A cat"), Some("nonexistent.png"), &Refs::default())
        .await
        .expect("image");
    assert_eq!(img.data, b64(b"fallback_data"));
}

#[tokio::test]
async fn deepseek_validate_uses_chat_completions_and_reports_success() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("authorization", "Bearer sk-ds-test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(chat_completion("OK")))
        .expect(1)
        .mount(&server)
        .await;
    let s = mocked(
        &server.uri(),
        Some(
            "ai_provider = deepseek\ndeepseek_api_key = sk-ds-test\ndeepseek_model = deepseek-chat\n",
        ),
        &[],
    );
    assert_eq!(s.current_provider(), Some(AiProviderId::Deepseek));
    assert_eq!(s.current_model().as_deref(), Some("deepseek-chat"));
    s.validate().await.expect("DeepSeek validation");
}

#[tokio::test]
async fn deepseek_validate_failure_is_generic() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(401).set_body_string("bad key"))
        .mount(&server)
        .await;
    let s = mocked(
        &server.uri(),
        Some("ai_provider = deepseek\ndeepseek_api_key = sk-bad\ndeepseek_model = deepseek-chat\n"),
        &[],
    );
    let err = s.validate().await.expect_err("401");
    assert_eq!(err.to_string(), "AI provider request failed");
    assert!(!err.to_string().contains("bad key"));
}

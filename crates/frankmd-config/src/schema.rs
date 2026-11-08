//! The `.fed` schema (`Config::SCHEMA`, backend.md §1.2). One row per key.

/// Value type of a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyType {
    /// Free string.
    String,
    /// Ruby `to_i` semantics.
    Integer,
    /// `true|1|yes|on` (case-insensitive) → true, anything else false.
    Boolean,
    /// JSON value (`recent_files`).
    Json,
    /// Validated custom theme JSON object.
    CustomTheme,
}

impl KeyType {
    /// Ruby schema type name (`string`, `integer`, `boolean`, `json`, `custom_theme`).
    pub fn as_str(self) -> &'static str {
        match self {
            KeyType::String => "string",
            KeyType::Integer => "integer",
            KeyType::Boolean => "boolean",
            KeyType::Json => "json",
            KeyType::CustomTheme => "custom_theme",
        }
    }
}

/// Static description of one config key.
#[derive(Debug, Clone, Copy)]
pub struct KeySpec {
    /// Key as written in `.fed`.
    pub name: &'static str,
    /// Value type.
    pub ty: KeyType,
    /// Default as it would be written in `.fed` (`None` = nil), e.g. `"14"`,
    /// `"true"`, `"[]"`; cast with [`crate::value::cast`].
    pub default: Option<&'static str>,
    /// ENV variable consulted when the key is absent from `.fed`.
    pub env: Option<&'static str>,
    /// Exposed to / writable from the Settings UI (`UI_KEYS`).
    pub ui: bool,
    /// Masked in logs and never sent to the UI (`SENSITIVE_KEYS`).
    pub secret: bool,
}

/// Typed handle for a schema key. Variants mirror backend.md §1.2 one-to-one,
/// **in schema order** (the order of [`Key::ALL`] is observable: `/logs/config`
/// entries, `all_settings`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[allow(missing_docs)]
pub enum Key {
    Theme,
    RecentFiles,
    CustomTheme,
    Locale,
    EditorFont,
    EditorFontSize,
    PreviewZoom,
    SidebarVisible,
    ViewMode,
    TypewriterMode,
    CursorRecenterOnClick,
    VimMode,
    ScrollSync,
    EditorIndent,
    EditorLineNumbers,
    ShowStats,
    EditorWidth,
    ImagesPath,
    MediaStorageProvider,
    ImageUploadExtensions,
    VideoUploadExtensions,
    AwsAccessKeyId,
    AwsSecretAccessKey,
    AwsS3Bucket,
    AwsRegion,
    AwsS3Endpoint,
    AwsS3ForcePathStyle,
    YoutubeApiKey,
    GoogleApiKey,
    GoogleCseId,
    AiProvider,
    AiModel,
    OllamaApiBase,
    OllamaModel,
    OpenrouterApiKey,
    OpenrouterModel,
    AnthropicApiKey,
    AnthropicModel,
    GeminiApiKey,
    GeminiModel,
    OpenaiApiKey,
    OpenaiModel,
    DeepseekApiKey,
    DeepseekModel,
    HugoPathStyle,
    AuthToken,
    ImageGenerationModel,
}

impl Key {
    /// Every key in schema order (47).
    pub const ALL: [Key; 47] = [
        Key::Theme,
        Key::RecentFiles,
        Key::CustomTheme,
        Key::Locale,
        Key::EditorFont,
        Key::EditorFontSize,
        Key::PreviewZoom,
        Key::SidebarVisible,
        Key::ViewMode,
        Key::TypewriterMode,
        Key::CursorRecenterOnClick,
        Key::VimMode,
        Key::ScrollSync,
        Key::EditorIndent,
        Key::EditorLineNumbers,
        Key::ShowStats,
        Key::EditorWidth,
        Key::ImagesPath,
        Key::MediaStorageProvider,
        Key::ImageUploadExtensions,
        Key::VideoUploadExtensions,
        Key::AwsAccessKeyId,
        Key::AwsSecretAccessKey,
        Key::AwsS3Bucket,
        Key::AwsRegion,
        Key::AwsS3Endpoint,
        Key::AwsS3ForcePathStyle,
        Key::YoutubeApiKey,
        Key::GoogleApiKey,
        Key::GoogleCseId,
        Key::AiProvider,
        Key::AiModel,
        Key::OllamaApiBase,
        Key::OllamaModel,
        Key::OpenrouterApiKey,
        Key::OpenrouterModel,
        Key::AnthropicApiKey,
        Key::AnthropicModel,
        Key::GeminiApiKey,
        Key::GeminiModel,
        Key::OpenaiApiKey,
        Key::OpenaiModel,
        Key::DeepseekApiKey,
        Key::DeepseekModel,
        Key::HugoPathStyle,
        Key::AuthToken,
        Key::ImageGenerationModel,
    ];

    /// Static spec for this key.
    pub fn spec(self) -> &'static KeySpec {
        // `SCHEMA` rows are in `Key::ALL` order, so the discriminant indexes it.
        &SCHEMA[self as usize]
    }

    /// `.fed` name (`spec().name`).
    pub fn name(self) -> &'static str {
        self.spec().name
    }

    /// Exact, case-sensitive lookup by schema name (`update("THEME" => …)` is
    /// skipped in the original). The `.fed` *parser* lowercases line keys
    /// before calling this (**[COMPAT]** `THEME = x` lines work).
    pub fn from_name(name: &str) -> Option<Key> {
        Key::ALL.into_iter().find(|k| k.name() == name)
    }

    /// The six `AI_CREDENTIAL_KEYS` that switch on file-only AI mode.
    pub const AI_CREDENTIALS: [Key; 6] = [
        Key::OllamaApiBase,
        Key::OpenrouterApiKey,
        Key::AnthropicApiKey,
        Key::GeminiApiKey,
        Key::OpenaiApiKey,
        Key::DeepseekApiKey,
    ];

    /// The 14 AI keys whose presence (even commented, `/^#?\s*key\s*=/i`)
    /// suppresses the AI-section upgrade (backend.md §1.6).
    pub const AI_UPGRADE_MARKERS: [Key; 14] = [
        Key::AiProvider,
        Key::AiModel,
        Key::OllamaApiBase,
        Key::OllamaModel,
        Key::OpenrouterApiKey,
        Key::OpenrouterModel,
        Key::AnthropicApiKey,
        Key::AnthropicModel,
        Key::GeminiApiKey,
        Key::GeminiModel,
        Key::OpenaiApiKey,
        Key::OpenaiModel,
        Key::DeepseekApiKey,
        Key::DeepseekModel,
    ];

    /// `UI_KEYS` in original order (17 keys).
    pub fn ui_keys() -> Vec<Key> {
        UI_KEYS.to_vec()
    }
}

/// **[COMPAT]** `Config::UI_KEYS`, in the original order (not schema order).
const UI_KEYS: [Key; 17] = [
    Key::Theme,
    Key::CustomTheme,
    Key::Locale,
    Key::EditorFont,
    Key::EditorFontSize,
    Key::PreviewZoom,
    Key::SidebarVisible,
    Key::ViewMode,
    Key::TypewriterMode,
    Key::CursorRecenterOnClick,
    Key::VimMode,
    Key::ScrollSync,
    Key::EditorIndent,
    Key::EditorLineNumbers,
    Key::ShowStats,
    Key::EditorWidth,
    Key::MediaStorageProvider,
];

/// Shorthand constructor for the table below.
const fn row(
    name: &'static str,
    ty: KeyType,
    default: Option<&'static str>,
    env: Option<&'static str>,
    ui: bool,
    secret: bool,
) -> KeySpec {
    KeySpec {
        name,
        ty,
        default,
        env,
        ui,
        secret,
    }
}

use KeyType::{Boolean as B, CustomTheme as C, Integer as I, Json as J, String as S};

/// **[COMPAT]** `Config::SCHEMA` (config.rb:11-79), one row per [`Key::ALL`] entry, same order.
/// Columns: name, type, default, ENV, UI, sensitive.
static SCHEMA: [KeySpec; 47] = [
    row("theme", S, None, None, true, false),
    row("recent_files", J, Some("[]"), None, false, false),
    row("custom_theme", C, None, None, true, false),
    row("locale", S, Some("en"), Some("FRANKMD_LOCALE"), true, false),
    row("editor_font", S, Some("cascadia-code"), None, true, false),
    row("editor_font_size", I, Some("14"), None, true, false),
    row("preview_zoom", I, Some("100"), None, true, false),
    row("sidebar_visible", B, Some("true"), None, true, false),
    row("view_mode", S, Some("split"), None, true, false),
    row("typewriter_mode", B, Some("false"), None, true, false),
    row(
        "cursor_recenter_on_click",
        B,
        Some("false"),
        None,
        true,
        false,
    ),
    row("vim_mode", B, Some("false"), None, true, false),
    row("scroll_sync", B, Some("true"), None, true, false),
    row("editor_indent", I, Some("2"), None, true, false),
    row("editor_line_numbers", I, Some("0"), None, true, false),
    row("show_stats", B, Some("false"), None, true, false),
    row("editor_width", I, Some("72"), None, true, false),
    row("images_path", S, None, Some("IMAGES_PATH"), false, false),
    row(
        "media_storage_provider",
        S,
        Some("local"),
        None,
        true,
        false,
    ),
    row(
        "image_upload_extensions",
        S,
        Some(".jpg,.jpeg,.png,.gif,.webp,.bmp"),
        Some("IMAGE_UPLOAD_EXTENSIONS"),
        false,
        false,
    ),
    row(
        "video_upload_extensions",
        S,
        Some(".mp4,.webm,.mkv,.mov,.avi,.m4v,.ogv"),
        Some("VIDEO_UPLOAD_EXTENSIONS"),
        false,
        false,
    ),
    row(
        "aws_access_key_id",
        S,
        None,
        Some("AWS_ACCESS_KEY_ID"),
        false,
        true,
    ),
    row(
        "aws_secret_access_key",
        S,
        None,
        Some("AWS_SECRET_ACCESS_KEY"),
        false,
        true,
    ),
    row(
        "aws_s3_bucket",
        S,
        None,
        Some("AWS_S3_BUCKET"),
        false,
        false,
    ),
    row("aws_region", S, None, Some("AWS_REGION"), false, false),
    row(
        "aws_s3_endpoint",
        S,
        None,
        Some("AWS_S3_ENDPOINT"),
        false,
        false,
    ),
    row(
        "aws_s3_force_path_style",
        B,
        Some("false"),
        Some("AWS_S3_FORCE_PATH_STYLE"),
        false,
        false,
    ),
    row(
        "youtube_api_key",
        S,
        None,
        Some("YOUTUBE_API_KEY"),
        false,
        true,
    ),
    row(
        "google_api_key",
        S,
        None,
        Some("GOOGLE_API_KEY"),
        false,
        true,
    ),
    row(
        "google_cse_id",
        S,
        None,
        Some("GOOGLE_CSE_ID"),
        false,
        false,
    ),
    row(
        "ai_provider",
        S,
        Some("auto"),
        Some("AI_PROVIDER"),
        false,
        false,
    ),
    row("ai_model", S, None, Some("AI_MODEL"), false, false),
    row(
        "ollama_api_base",
        S,
        None,
        Some("OLLAMA_API_BASE"),
        false,
        false,
    ),
    row(
        "ollama_model",
        S,
        Some("llama3.2:latest"),
        Some("OLLAMA_MODEL"),
        false,
        false,
    ),
    row(
        "openrouter_api_key",
        S,
        None,
        Some("OPENROUTER_API_KEY"),
        false,
        true,
    ),
    row(
        "openrouter_model",
        S,
        Some("openai/gpt-4o-mini"),
        Some("OPENROUTER_MODEL"),
        false,
        false,
    ),
    row(
        "anthropic_api_key",
        S,
        None,
        Some("ANTHROPIC_API_KEY"),
        false,
        true,
    ),
    row(
        "anthropic_model",
        S,
        Some("claude-sonnet-4-20250514"),
        Some("ANTHROPIC_MODEL"),
        false,
        false,
    ),
    row(
        "gemini_api_key",
        S,
        None,
        Some("GEMINI_API_KEY"),
        false,
        true,
    ),
    row(
        "gemini_model",
        S,
        Some("gemini-2.0-flash"),
        Some("GEMINI_MODEL"),
        false,
        false,
    ),
    row(
        "openai_api_key",
        S,
        None,
        Some("OPENAI_API_KEY"),
        false,
        true,
    ),
    row(
        "openai_model",
        S,
        Some("gpt-4o-mini"),
        Some("OPENAI_MODEL"),
        false,
        false,
    ),
    row(
        "deepseek_api_key",
        S,
        None,
        Some("DEEPSEEK_API_KEY"),
        false,
        true,
    ),
    row(
        "deepseek_model",
        S,
        Some("deepseek-chat"),
        Some("DEEPSEEK_MODEL"),
        false,
        false,
    ),
    row(
        "hugo_path_style",
        S,
        Some("dated"),
        Some("HUGO_PATH_STYLE"),
        false,
        false,
    ),
    row(
        "auth_token",
        S,
        None,
        Some("FRANKMD_AUTH_TOKEN"),
        false,
        true,
    ),
    row(
        "image_generation_model",
        S,
        Some("google/gemini-3.1-flash-image-preview"),
        Some("IMAGE_GENERATION_MODEL"),
        false,
        false,
    ),
];

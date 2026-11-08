//! `.fed > ENV > default` resolution with provenance.

use crate::schema::KeyType;
use crate::value::{cast, present_str};
use crate::{ConfigError, EnvSnapshot, Key, Value, fed};
use secrecy::SecretString;

/// Where a resolved value came from (shown in Settings: "set by ENV", ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    /// Key present in `.fed` (possibly with an empty value → `None`).
    Fed,
    /// Key present in the global `config.toml` (possibly empty → `None`).
    Global,
    /// Taken from the named ENV variable.
    Env(&'static str),
    /// Schema default.
    Default,
}

impl Provenance {
    /// **[COMPAT]** `LogsController#config_source` label: `file`, `env`, `default`.
    pub fn as_str(self) -> &'static str {
        match self {
            Provenance::Fed => "file",
            Provenance::Global => "global",
            Provenance::Env(_) => "env",
            Provenance::Default => "default",
        }
    }
}

/// A value plus its provenance.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    /// The value (`None` = nil).
    pub value: Option<Value>,
    /// Which layer supplied it.
    pub source: Provenance,
}

/// AI providers known to the original app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum AiProviderId {
    Openai,
    Deepseek,
    Anthropic,
    Openrouter,
    Ollama,
    Gemini,
}

impl AiProviderId {
    /// **[COMPAT]** `AI_PROVIDER_PRIORITY` (auto selection order).
    pub const PRIORITY: [AiProviderId; 6] = [
        AiProviderId::Openai,
        AiProviderId::Deepseek,
        AiProviderId::Anthropic,
        AiProviderId::Openrouter,
        AiProviderId::Ollama,
        AiProviderId::Gemini,
    ];

    /// **[COMPAT]** order of `ai_providers_available` (`available_providers`).
    pub const AVAILABILITY_ORDER: [AiProviderId; 6] = [
        AiProviderId::Ollama,
        AiProviderId::Openrouter,
        AiProviderId::Deepseek,
        AiProviderId::Anthropic,
        AiProviderId::Gemini,
        AiProviderId::Openai,
    ];

    /// Id as used in `.fed`/JSON (`openai`, ...).
    pub fn as_str(self) -> &'static str {
        match self {
            AiProviderId::Openai => "openai",
            AiProviderId::Deepseek => "deepseek",
            AiProviderId::Anthropic => "anthropic",
            AiProviderId::Openrouter => "openrouter",
            AiProviderId::Ollama => "ollama",
            AiProviderId::Gemini => "gemini",
        }
    }

    /// Parse an id (exact, lowercase). `auto` is not a provider → `None`.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::PRIORITY.into_iter().find(|p| p.as_str() == name)
    }

    /// The credential key that makes this provider available.
    pub fn credential_key(self) -> Key {
        match self {
            AiProviderId::Openai => Key::OpenaiApiKey,
            AiProviderId::Deepseek => Key::DeepseekApiKey,
            AiProviderId::Anthropic => Key::AnthropicApiKey,
            AiProviderId::Openrouter => Key::OpenrouterApiKey,
            AiProviderId::Ollama => Key::OllamaApiBase,
            AiProviderId::Gemini => Key::GeminiApiKey,
        }
    }

    /// The `<provider>_model` key.
    pub fn model_key(self) -> Key {
        match self {
            AiProviderId::Openai => Key::OpenaiModel,
            AiProviderId::Deepseek => Key::DeepseekModel,
            AiProviderId::Anthropic => Key::AnthropicModel,
            AiProviderId::Openrouter => Key::OpenrouterModel,
            AiProviderId::Ollama => Key::OllamaModel,
            AiProviderId::Gemini => Key::GeminiModel,
        }
    }
}

/// `feature_available?` features.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum Feature {
    S3Upload,
    YoutubeSearch,
    GoogleSearch,
    LocalImages,
    Ai,
}

/// One `/logs/config` row (config introspection).
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigEntry {
    /// The key.
    pub key: Key,
    /// JSON value; `"****"` for a non-nil sensitive value.
    pub value: serde_json::Value,
    /// Source layer.
    pub source: Provenance,
    /// ENV variable name, if the key has one.
    pub env_var: Option<&'static str>,
    /// In `SENSITIVE_KEYS`.
    pub sensitive: bool,
}

/// An immutable, fully resolved view of the configuration for one vault.
/// Rebuilt (never mutated) whenever `.fed` changes on disk or is written.
///
/// `Debug` masks the values of sensitive keys.
#[derive(Clone)]
pub struct ResolvedConfig {
    file: Vec<(Key, Option<Value>)>,
    global: Vec<(Key, Option<Value>)>,
    env: EnvSnapshot,
    /// Problems found while loading (skipped lines, undecodable file, ...),
    /// surfaced in the UI. Each names the offending key when there is one.
    pub warnings: Vec<String>,
}

impl std::fmt::Debug for ResolvedConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let masked = |layer: &[(Key, Option<Value>)]| -> Vec<(&'static str, String)> {
            layer
                .iter()
                .map(|(k, v)| {
                    let shown = match v {
                        Some(_) if k.spec().secret => "****".to_owned(),
                        other => format!("{other:?}"),
                    };
                    (k.name(), shown)
                })
                .collect()
        };
        f.debug_struct("ResolvedConfig")
            .field("file", &masked(&self.file))
            .field("global", &masked(&self.global))
            .field("env", &self.env)
            .field("warnings", &self.warnings)
            .finish()
    }
}

/// Generic Rails `present?` for a resolved value.
fn is_present(v: Option<&Value>) -> bool {
    match v {
        None => false,
        Some(Value::Str(s)) => present_str(s),
        Some(Value::Bool(b)) => *b,
        Some(Value::Json(j)) => match j {
            serde_json::Value::Null | serde_json::Value::Bool(false) => false,
            serde_json::Value::String(s) => present_str(s),
            serde_json::Value::Array(a) => !a.is_empty(),
            serde_json::Value::Object(o) => !o.is_empty(),
            _ => true,
        },
        Some(Value::Int(_) | Value::CustomTheme(_)) => true,
    }
}

/// The value as a present string (`presence` on a string value).
fn present_string(v: Option<Value>) -> Option<String> {
    match v {
        Some(Value::Str(s)) if present_str(&s) => Some(s),
        _ => None,
    }
}

/// The schema default of `key`, cast.
fn default_of(key: Key) -> Option<Value> {
    let spec = key.spec();
    spec.default.and_then(|d| cast(d, spec.ty).ok().flatten())
}

impl ResolvedConfig {
    /// Build from `.fed` text (or `None` if absent) and an ENV snapshot.
    pub fn new(fed_text: Option<&str>, env: EnvSnapshot) -> Self {
        let parsed = fed_text.map(fed::parse).unwrap_or_default();
        Self {
            file: parsed.values,
            global: Vec::new(),
            env,
            warnings: parsed.warnings,
        }
    }

    /// Build from raw `.fed` bytes. Invalid UTF-8 → no file values at all plus
    /// a warning (**[COMPAT]**: Ruby's parse raises and `@values = {}`).
    pub fn from_bytes(fed: Option<&[u8]>, env: EnvSnapshot) -> Self {
        match fed.map(std::str::from_utf8) {
            None => Self::new(None, env),
            Some(Ok(text)) => Self::new(Some(text), env),
            Some(Err(_)) => Self::failed(
                env,
                "Failed to load .fed config: invalid byte sequence in UTF-8".to_owned(),
            ),
        }
    }

    /// No file values, one warning (the file could not be read or decoded).
    pub(crate) fn failed(env: EnvSnapshot, warning: String) -> Self {
        Self {
            file: Vec::new(),
            global: Vec::new(),
            env,
            warnings: vec![warning],
        }
    }

    /// Attach the global layer (see [`crate::global::load_global`]); sits
    /// between `.fed` and ENV.
    #[must_use]
    pub fn with_global(mut self, values: Vec<(Key, Option<Value>)>) -> Self {
        self.global = values;
        self
    }

    /// Keys present in the global config, in file order.
    pub fn global_values(&self) -> &[(Key, Option<Value>)] {
        &self.global
    }

    /// Key present in this vault's `.fed` (even with an empty value).
    pub fn in_vault_file(&self, key: Key) -> bool {
        self.file_value(key).is_some()
    }

    /// Key present in the global config (even with an empty value).
    pub fn in_global_file(&self, key: Key) -> bool {
        self.global_value(key).is_some()
    }

    /// The ENV snapshot this view was built with.
    pub fn env(&self) -> &EnvSnapshot {
        &self.env
    }

    /// Keys present in `.fed` (Ruby `@values`), in first-insertion order.
    pub fn file_values(&self) -> &[(Key, Option<Value>)] {
        &self.file
    }

    /// The `.fed` entry for `key`, if the key is present (value may be nil).
    fn file_value(&self, key: Key) -> Option<&Option<Value>> {
        self.file.iter().find(|(k, _)| *k == key).map(|(_, v)| v)
    }

    fn global_value(&self, key: Key) -> Option<&Option<Value>> {
        self.global.iter().find(|(k, _)| *k == key).map(|(_, v)| v)
    }

    /// **[COMPAT]** `Config#get`, with the global layer between `.fed` and ENV.
    pub fn get(&self, key: Key) -> Resolved {
        if let Some(v) = self.file_value(key) {
            return Resolved {
                value: v.clone(),
                source: Provenance::Fed,
            };
        }
        if let Some(v) = self.global_value(key) {
            return Resolved {
                value: v.clone(),
                source: Provenance::Global,
            };
        }
        let spec = key.spec();
        if let Some(var) = spec.env
            && let Some(raw) = self.env.present(var)
        {
            return Resolved {
                value: cast(raw, spec.ty).ok().flatten(),
                source: Provenance::Env(var),
            };
        }
        Resolved {
            value: default_of(key),
            source: Provenance::Default,
        }
    }

    /// `get(key).value`.
    pub fn value(&self, key: Key) -> Option<Value> {
        self.get(key).value
    }

    /// **[COMPAT]** `Config#get_ai` (file-only mode when any AI credential key
    /// is present in `.fed`: ENV ignored, `.fed` value or schema default).
    pub fn get_ai(&self, key: Key) -> Resolved {
        if !self.ai_configured_in_file() {
            return self.get(key);
        }
        match self.file_value(key) {
            Some(v) => Resolved {
                value: v.clone(),
                source: Provenance::Fed,
            },
            None => match self.global_value(key) {
                Some(v) => Resolved {
                    value: v.clone(),
                    source: Provenance::Global,
                },
                None => Resolved {
                    value: default_of(key),
                    source: Provenance::Default,
                },
            },
        }
    }

    /// **[COMPAT]** `ai_configured_in_file?`: any of [`Key::AI_CREDENTIALS`]
    /// present in `.fed`, even with an empty value. Extension: the global
    /// config counts as "file" too, so a user who keeps their AI credentials
    /// in `config.toml` gets the same ENV-ignoring file-only mode; lookups in
    /// that mode are `.fed > global > default`.
    pub fn ai_configured_in_file(&self) -> bool {
        Key::AI_CREDENTIALS
            .iter()
            .any(|k| self.file_value(*k).is_some() || self.global_value(*k).is_some())
    }

    /// **[COMPAT]** `ai_providers_available` (order: ollama, openrouter,
    /// deepseek, anthropic, gemini, openai).
    pub fn ai_providers_available(&self) -> Vec<AiProviderId> {
        AiProviderId::AVAILABILITY_ORDER
            .into_iter()
            .filter(|p| is_present(self.get_ai(p.credential_key()).value.as_ref()))
            .collect()
    }

    /// **[COMPAT]** `effective_ai_provider` (priority openai > deepseek >
    /// anthropic > openrouter > ollama > gemini).
    pub fn effective_ai_provider(&self) -> Option<AiProviderId> {
        let available = self.ai_providers_available();
        if available.is_empty() {
            return None;
        }
        let configured = present_string(self.get_ai(Key::AiProvider).value)
            .and_then(|name| AiProviderId::from_name(&name))
            .filter(|p| available.contains(p));
        configured.or_else(|| {
            AiProviderId::PRIORITY
                .into_iter()
                .find(|p| available.contains(p))
        })
    }

    /// **[COMPAT]** `effective_ai_model`: `get_ai(ai_model)` if present, else
    /// `get_ai(<provider>_model)`; `None` without a provider.
    pub fn effective_ai_model(&self) -> Option<String> {
        let provider = self.effective_ai_provider()?;
        present_string(self.get_ai(Key::AiModel).value).or_else(|| {
            match self.get_ai(provider.model_key()).value {
                Some(Value::Str(s)) => Some(s),
                _ => None,
            }
        })
    }

    /// **[COMPAT]** image generation bypasses file-only mode
    /// (`@values["openrouter_api_key"] || ENV["OPENROUTER_API_KEY"]`; an empty
    /// file value is nil and falls through; a blank ENV value disables).
    pub fn image_generation_key(&self) -> Option<SecretString> {
        if let Some(Some(Value::Str(key))) = self.file_value(Key::OpenrouterApiKey) {
            return Some(SecretString::from(key.clone()));
        }
        if let Some(Some(Value::Str(key))) = self.global_value(Key::OpenrouterApiKey) {
            return Some(SecretString::from(key.clone()));
        }
        self.env
            .present("OPENROUTER_API_KEY")
            .map(|key| SecretString::from(key.to_owned()))
    }

    /// **[COMPAT]** `upload_extensions`: split on `,`, trim, downcase, drop blanks.
    pub fn upload_extensions(&self, key: Key) -> Vec<String> {
        let raw = match self.value(key) {
            Some(Value::Str(s)) => s,
            _ => String::new(),
        };
        raw.split(',')
            .map(|e| crate::value::ruby_strip(e).to_lowercase())
            .filter(|e| present_str(e))
            .collect()
    }

    /// **[COMPAT]** `UploadStorage.s3_configured?`: access key, secret and bucket
    /// present, and `aws_s3_endpoint` (if set) a valid http(s) URL with a host.
    pub fn s3_configured(&self) -> bool {
        let creds = [
            Key::AwsAccessKeyId,
            Key::AwsSecretAccessKey,
            Key::AwsS3Bucket,
        ];
        if !creds.iter().all(|k| is_present(self.value(*k).as_ref())) {
            return false;
        }
        match present_string(self.value(Key::AwsS3Endpoint)) {
            None => true,
            Some(endpoint) => crate::uri::is_plain_http_url(&endpoint),
        }
    }

    /// **[COMPAT]** `feature_available?`.
    pub fn feature_available(&self, feature: Feature) -> bool {
        let present = |k: Key| is_present(self.value(k).as_ref());
        match feature {
            Feature::S3Upload => self.s3_configured(),
            Feature::YoutubeSearch => present(Key::YoutubeApiKey),
            Feature::GoogleSearch => present(Key::GoogleApiKey) && present(Key::GoogleCseId),
            Feature::LocalImages => present(Key::ImagesPath),
            Feature::Ai => !self.ai_providers_available().is_empty(),
        }
    }

    /// **[COMPAT]** `ui_settings`: every `UI_KEYS` key with its resolved value.
    pub fn ui_settings(&self) -> Vec<(Key, Option<Value>)> {
        Key::ui_keys()
            .into_iter()
            .map(|k| (k, self.value(k)))
            .collect()
    }

    /// **[COMPAT]** `all_settings` (masked): sensitive keys appear as
    /// `<key>_configured: bool`, everything else as its JSON value; schema order.
    pub fn all_settings_masked(&self) -> Vec<(String, serde_json::Value)> {
        Key::ALL
            .into_iter()
            .map(|k| {
                let v = self.value(k);
                if k.spec().secret {
                    (
                        format!("{}_configured", k.name()),
                        serde_json::Value::Bool(is_present(v.as_ref())),
                    )
                } else {
                    (k.name().to_owned(), json_of(v.as_ref()))
                }
            })
            .collect()
    }

    /// **[COMPAT]** `/logs/config` entries, one per schema key in order.
    pub fn entries(&self) -> Vec<ConfigEntry> {
        Key::ALL
            .into_iter()
            .map(|key| {
                let Resolved { value, source } = self.get(key);
                let spec = key.spec();
                let value = if spec.secret && is_present(value.as_ref()) {
                    serde_json::Value::String("****".to_owned())
                } else {
                    json_of(value.as_ref())
                };
                ConfigEntry {
                    key,
                    value,
                    source,
                    env_var: spec.env,
                    sensitive: spec.secret,
                }
            })
            .collect()
    }

    /// **[COMPAT]** `format_value`: the canonical `key = value` line for a
    /// write-back, validating against this config (S3 must be configured for
    /// `media_storage_provider = s3`).
    pub fn format_line(&self, key: Key, value: Option<&Value>) -> Result<String, ConfigError> {
        let name = key.name();
        if key == Key::MediaStorageProvider {
            match value.and_then(Value::as_str) {
                Some("local") => {}
                Some("s3") if self.s3_configured() => {}
                Some("s3") => return Err(ConfigError::S3NotConfigured),
                _ => return Err(ConfigError::UnknownMediaProvider),
            }
        }
        Ok(match key.spec().ty {
            KeyType::Json | KeyType::CustomTheme => {
                let json = match value {
                    Some(Value::CustomTheme(t)) => t.to_json_string(),
                    other => json_of(other).to_string(),
                };
                format!("{name} = {json}")
            }
            KeyType::String => {
                let text = value.map(ruby_to_s).unwrap_or_default();
                if text.contains(['"', '\r', '\n']) {
                    return Err(ConfigError::UnsafeString { key });
                }
                if text.contains([' ', '=']) {
                    format!("{name} = \"{text}\"")
                } else {
                    format!("{name} = {text}")
                }
            }
            KeyType::Boolean => {
                // Ruby truthiness: only nil and false are false.
                let truthy = !matches!(value, None | Some(Value::Bool(false)));
                format!("{name} = {truthy}")
            }
            KeyType::Integer => format!("{name} = {}", value.map(ruby_to_s).unwrap_or_default()),
        })
    }
}

/// JSON of an optional value (`nil` → `null`).
fn json_of(v: Option<&Value>) -> serde_json::Value {
    v.map_or(serde_json::Value::Null, Value::to_json)
}

/// Ruby `to_s` of a typed value, as interpolated into a `.fed` line.
fn ruby_to_s(v: &Value) -> String {
    match v {
        Value::Str(s) => s.clone(),
        Value::Int(i) => i.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Json(j) => j.to_string(),
        Value::CustomTheme(t) => t.to_json_string(),
    }
}

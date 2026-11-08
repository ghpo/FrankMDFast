//! Port of the `Config` introspection half of
//! `test/controllers/logs_controller_test.rb` (12 tests: 5 ported, 7 skipped).
//!
//! `GET /logs/config` ↦ `ConfigStore::path()` + `ResolvedConfig::entries()`.
#![allow(clippy::unwrap_used)]

mod common;

use common::*;
use frankmd_config::{Key, Provenance};
use serde_json::json;

// skip: test/controllers/logs_controller_test.rb:35  "tail returns JSON with environment and file info" [P] Rails log file tail
// skip: test/controllers/logs_controller_test.rb:47  "tail respects lines parameter"                   [P]
// skip: test/controllers/logs_controller_test.rb:59  "tail limits lines to 500 max"                    [P]
// skip: test/controllers/logs_controller_test.rb:69  "tail handles empty log file"                     [P]
// skip: test/controllers/logs_controller_test.rb:82  "tail handles missing log file"                   [P]
// skip: test/controllers/logs_controller_test.rb:106 "tail handles invalid lines parameter"            [P]
// skip: test/controllers/logs_controller_test.rb:178 "tail returns lines as array of strings"          [P]

// port: test/controllers/logs_controller_test.rb:117 "config returns JSON with config file info and entries"
#[test]
fn config_returns_json_with_config_file_info_and_entries() {
    let v = TestVault::new();
    let store = v.open_empty_env();
    assert_eq!(store.path(), v.fed_path());
    assert!(store.path().is_file(), "config_file_exists");
    assert!(!store.current().ai_configured_in_file());
    assert!(!store.current().entries().is_empty());
}

// port: test/controllers/logs_controller_test.rb:129 "config entries include all SCHEMA keys"
#[test]
fn config_entries_include_all_schema_keys() {
    let entries = TestVault::new().open_empty_env().current().entries();
    let keys: Vec<Key> = entries.iter().map(|e| e.key).collect();
    // Schema order (the log viewer lists them in that order).
    assert_eq!(keys, Key::ALL.to_vec());
}

// port: test/controllers/logs_controller_test.rb:139 "config entries have expected fields"
#[test]
fn config_entries_have_expected_fields() {
    let entries = TestVault::new().open_empty_env().current().entries();
    let first = &entries[0];
    assert_eq!(first.key, Key::Theme);
    assert!(["file", "env", "default"].contains(&first.source.as_str()));
    for e in &entries {
        assert_eq!(e.env_var, e.key.spec().env);
        assert_eq!(e.sensitive, e.key.spec().secret);
    }
}

// port: test/controllers/logs_controller_test.rb:152 "config masks sensitive values"
#[test]
fn config_masks_sensitive_values() {
    // Give every sensitive key a value so the mask is actually exercised.
    let v = TestVault::with_fed(
        "auth_token = t\naws_access_key_id = a\naws_secret_access_key = b\nyoutube_api_key = c\n\
         google_api_key = d\nopenai_api_key = e\nopenrouter_api_key = f\nanthropic_api_key = g\ngemini_api_key = h\ndeepseek_api_key = i\n",
    );
    for e in v
        .open_empty_env()
        .current()
        .entries()
        .iter()
        .filter(|e| e.sensitive)
    {
        assert_eq!(
            e.value,
            json!("****"),
            "Sensitive key {:?} should be masked",
            e.key
        );
    }
}

// port: test/controllers/logs_controller_test.rb:164 "config source reflects ENV when ENV is set"
#[test]
fn config_source_reflects_env_when_env_is_set() {
    let store = TestVault::new().open(env(&[("YOUTUBE_API_KEY", "test-yt-key-12345678")]));
    let entries = store.current().entries();
    let yt = entries
        .iter()
        .find(|e| e.key == Key::YoutubeApiKey)
        .unwrap();
    assert_eq!(yt.source, Provenance::Env("YOUTUBE_API_KEY"));
    assert_eq!(yt.value, json!("****"));
}

//! Golden conformance: `config_parse.json`, `config_env.json`,
//! `config_writeback.json`, `config_template.json` and `fed_template.fed`
//! (captured from the original `app/models/config.rb`, see tools/golden/ruby/config.rb).
//!
//! Decisions (also in docs/testing/port-config-theme.md):
//! - Rails log lines are not compared, except that every `Ignoring invalid
//!   setting <key>` / `Failed to load` must surface as a `warnings` entry.
//! - Ruby `instance_values` (the possibly stale in-memory `@values`) is not
//!   replicated: a `ConfigStore` snapshot is always re-parsed from disk after
//!   a write. Each step is therefore checked against the next fixture value
//!   that reflects disk (`reload` result, else `reread_file_values`, else
//!   `instance_values` when the two agree by construction).
//! - Unknown key names (`bogus`, `THEME`) cannot be expressed as `Key`; they
//!   are checked as `Key::from_name(..) == None` and the op is a no-op.
//! - `generate_config_lines` is dead code in the original and is skipped.
//! - Inputs that depend on the process cwd (`NOTES_PATH=""`) are skipped.
#![allow(clippy::unwrap_used)]

mod common;

use common::*;
use frankmd_config::value::coerce;
use frankmd_config::{AiProviderId, Feature, Key, ResolvedConfig, Value, dirs, fed};
use frankmd_testkit::{Case, assert_cases, golden};
use serde::Deserialize;
use serde_json::{Value as J, json};
use std::path::Path;

fn fed_input_bytes(input: &J) -> Option<Vec<u8>> {
    if let Some(b64) = input.get("fed_base64").and_then(J::as_str) {
        return Some(base64_decode(b64));
    }
    input
        .get("fed")
        .and_then(J::as_str)
        .map(|s| s.as_bytes().to_vec())
}

fn check_warnings(rc: &ResolvedConfig, log: &J) -> Result<(), String> {
    for line in log.as_array().into_iter().flatten().filter_map(J::as_str) {
        if let Some(k) = line.strip_prefix("WARN: Ignoring invalid setting ")
            && !rc.warnings.iter().any(|w| w.contains(k))
        {
            return Err(format!("missing warning for {k}: {:?}", rc.warnings));
        }
        if line.contains("Failed to load .fed config") && rc.warnings.is_empty() {
            return Err(format!("expected a load warning for {line:?}"));
        }
    }
    Ok(())
}

// ------------------------------------------------------------------ config_parse

#[test]
fn golden_config_parse_values() {
    let g = golden("config_parse");
    assert_cases("config_parse", &g.cases, |c| {
        let rc = ResolvedConfig::from_bytes(
            fed_input_bytes(&c.input).as_deref(),
            env_json(&c.input["env"], "/R"),
        );
        eq(
            "file_values",
            &file_values_json(&rc),
            &c.expected["file_values"],
        )?;
        eq("resolved", &resolved_json(&rc), &c.expected["resolved"])?;
        check_warnings(&rc, &c.expected["log"])
    });
}

#[test]
fn golden_config_parse_load_rewrites_file() {
    let g = golden("config_parse");
    assert_cases("config_parse (fed_after_load)", &g.cases, |c| {
        let input = fed_input_bytes(&c.input).unwrap();
        let v = TestVault::new();
        v.write_fed(&input);
        let store = v.open(env_json(&c.input["env"], "/R"));
        let want: Vec<u8> = match &c.expected["fed_after_load"] {
            J::String(s) if s == "<UNCHANGED>" => input.clone(),
            J::String(s) => s.as_bytes().to_vec(),
            J::Object(o) => base64_decode(o["base64"].as_str().unwrap()),
            other => return Err(format!("unexpected fed_after_load {other}")),
        };
        let got = v.read_fed_bytes().unwrap_or_default();
        if got != want {
            return Err(format!(
                "fed after load:\n    got  {:?}\n    want {:?}",
                String::from_utf8_lossy(&got),
                String::from_utf8_lossy(&want)
            ));
        }
        eq(
            "resolved (store)",
            &resolved_json(&store.current()),
            &c.expected["resolved"],
        )
    });
}

// ------------------------------------------------------------------ config_env

fn feature(name: &str) -> Option<Feature> {
    Some(match name {
        "s3_upload" => Feature::S3Upload,
        "youtube_search" => Feature::YoutubeSearch,
        "google_search" => Feature::GoogleSearch,
        "local_images" => Feature::LocalImages,
        "ai" => Feature::Ai,
        _ => return None,
    })
}

#[test]
fn golden_config_env_precedence() {
    let g = golden("config_env");
    assert_cases("config_env precedence", g.cases_for("precedence"), |c| {
        let fed = c.input["fed"].as_str();
        let rc = ResolvedConfig::new(fed, env_json(&c.input["env"], "/R"));
        let e = &c.expected;
        eq("file_values", &file_values_json(&rc), &e["file_values"])?;
        eq("resolved", &resolved_json(&rc), &e["resolved"])?;
        let sources: J = Key::ALL
            .iter()
            .map(|k| (k.name().to_owned(), json!(rc.get(*k).source.as_str())))
            .collect();
        eq("sources", &sources, &e["sources"])?;
        let get_ai: J = e["get_ai"]
            .as_object()
            .unwrap()
            .keys()
            .map(|n| (n.clone(), opt_json(rc.get_ai(key(n)).value.as_ref())))
            .collect();
        eq("get_ai", &get_ai, &e["get_ai"])?;
        eq(
            "ai_configured_in_file",
            &json!(rc.ai_configured_in_file()),
            &e["ai_configured_in_file"],
        )?;
        let avail: Vec<&str> = rc
            .ai_providers_available()
            .into_iter()
            .map(AiProviderId::as_str)
            .collect();
        eq(
            "ai_providers_available",
            &json!(avail),
            &e["ai_providers_available"],
        )?;
        eq(
            "effective_ai_provider",
            &json!(rc.effective_ai_provider().map(AiProviderId::as_str)),
            &e["effective_ai_provider"],
        )?;
        eq(
            "effective_ai_model",
            &json!(rc.effective_ai_model()),
            &e["effective_ai_model"],
        )?;
        for (name, want) in e["features"].as_object().unwrap() {
            // `bogus` → false in Ruby; unrepresentable as a typed Feature.
            let got = feature(name).is_some_and(|f| rc.feature_available(f));
            eq(&format!("features.{name}"), &json!(got), want)?;
        }
        let masked: J = rc.all_settings_masked().into_iter().collect();
        eq("all_settings_masked", &masked, &e["all_settings_masked"])?;
        let ext = json!({
            "image_upload_extensions": rc.upload_extensions(Key::ImageUploadExtensions),
            "video_upload_extensions": rc.upload_extensions(Key::VideoUploadExtensions),
        });
        eq("upload_extensions", &ext, &e["upload_extensions"])
    });
}

#[test]
fn golden_config_env_notes_root() {
    let g = golden("config_env");
    assert_cases("config_env Config.new", g.cases_for("Config.new"), |c| {
        let root = "/golden/R";
        let env = env_json(&c.input["env"], root);
        let explicit = c
            .input
            .get("base_path")
            .and_then(J::as_str)
            .map(|p| p.replace("<ROOT>", root));
        let got = dirs::notes_root(explicit.as_deref().map(Path::new), &env).unwrap();
        let want = c.expected["base_path"]
            .as_str()
            .unwrap()
            .replace("<ROOT>", root);
        eq("base_path", &json!(got.to_string_lossy()), &json!(want))?;
        if let Some(cfp) = c.expected.get("config_file_path").and_then(J::as_str) {
            let got = dirs::config_file_path(&got);
            eq(
                "config_file_path",
                &json!(got.to_string_lossy()),
                &json!(cfp.replace("<ROOT>", root)),
            )?;
        }
        Ok(())
    });
}

#[test]
fn golden_config_env_images_initializer() {
    let g = golden("config_env");
    assert_cases(
        "config_env images initializer",
        g.cases_for("config/initializers/images.rb"),
        |c| {
            let root = "/golden/home";
            let env = env_json(&c.input["env"], root);
            let has_pictures = c.input["home_has_Pictures"].as_bool().unwrap();
            let pictures = format!("{root}/Pictures");
            let out = env.with_images_path_default(|p| has_pictures && p == Path::new(&pictures));
            let want = c.expected["IMAGES_PATH"]
                .as_str()
                .map(|s| s.replace("<ROOT>", root));
            eq("IMAGES_PATH", &json!(out.raw("IMAGES_PATH")), &json!(want))
        },
    );
}

#[test]
fn golden_config_env_openrouter_key_for_images() {
    use secrecy::ExposeSecret as _;
    let g = golden("config_env");
    assert_cases(
        "config_env openrouter_key_for_images",
        g.cases_for("AiService.openrouter_key_for_images (expression)"),
        |c| {
            let rc = ResolvedConfig::new(c.input["fed"].as_str(), env_json(&c.input["env"], "/R"));
            let key = rc
                .image_generation_key()
                .map(|k| k.expose_secret().to_owned());
            eq(
                "openrouter_key_for_images",
                &json!(key),
                &c.expected["openrouter_key_for_images"],
            )?;
            eq(
                "get_ai(openrouter_api_key)",
                &opt_json(rc.get_ai(Key::OpenrouterApiKey).value.as_ref()),
                &c.expected["get_ai_openrouter_api_key"],
            )
        },
    );
}

#[test]
fn golden_config_env_schema_table() {
    let g: J = serde_json::from_slice(&frankmd_testkit::golden_bytes("config_env.json")).unwrap();
    let rows = g["keys"].as_array().unwrap();
    let names: Vec<&str> = Key::ALL.iter().map(|k| k.name()).collect();
    let want_names: Vec<&str> = rows.iter().map(|r| r["key"].as_str().unwrap()).collect();
    assert_eq!(names, want_names, "Key::ALL must be the schema in order");
    let mut failures = Vec::new();
    for r in rows {
        let k = key(r["key"].as_str().unwrap());
        let spec = k.spec();
        let default = spec
            .default
            .map(|d| frankmd_config::value::cast(d, spec.ty).unwrap());
        let got = json!({
            "key": spec.name,
            "type": spec.ty.as_str(),
            "default": opt_json(default.flatten().as_ref()),
            "env": spec.env,
            "sensitive": spec.secret,
            "ui": spec.ui,
            "ai_credential": Key::AI_CREDENTIALS.contains(&k),
        });
        if &got != r {
            failures.push(format!("{}: got {got} want {r}", spec.name));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));

    let consts = &g["config_constants"];
    let ui: Vec<&str> = Key::ui_keys().iter().map(|k| k.name()).collect();
    assert_eq!(json!(ui), consts["UI_KEYS"]);
    let mut sensitive: Vec<&str> = Key::ALL
        .iter()
        .filter(|k| k.spec().secret)
        .map(|k| k.name())
        .collect();
    let mut want_sensitive: Vec<&str> = consts["SENSITIVE_KEYS"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    sensitive.sort_unstable();
    want_sensitive.sort_unstable();
    assert_eq!(sensitive, want_sensitive);
    let creds: Vec<&str> = Key::AI_CREDENTIALS.iter().map(|k| k.name()).collect();
    assert_eq!(json!(creds), consts["AI_CREDENTIAL_KEYS"]);
    let prio: Vec<&str> = AiProviderId::PRIORITY.iter().map(|p| p.as_str()).collect();
    assert_eq!(json!(prio), consts["AI_PROVIDER_PRIORITY"]);
    let order: Vec<&str> = AiProviderId::AVAILABILITY_ORDER
        .iter()
        .map(|p| p.as_str())
        .collect();
    assert_eq!(json!(order), consts["ai_providers_available_order"]);
    assert_eq!(
        consts["CONFIG_FILE"],
        json!(frankmd_config::store::CONFIG_FILE)
    );
    assert_eq!(consts["LOCK_FILE"], json!(frankmd_fs::lock::CONFIG_LOCK));
}

// ------------------------------------------------------------------ template

#[test]
fn golden_fed_template_is_byte_identical() {
    assert_eq!(
        fed::template().as_bytes(),
        frankmd_testkit::golden_bytes("fed_template.fed").as_slice()
    );
    assert_eq!(fed::template().len(), 3136);
}

#[test]
fn golden_template_sections() {
    let g: J =
        serde_json::from_slice(&frankmd_testkit::golden_bytes("config_template.json")).unwrap();
    let got: J = fed::template_sections()
        .iter()
        .map(|s| json!({"marker": s.marker, "lines": s.lines}))
        .collect();
    assert_eq!(got, g["template_sections"]);
    let last = g["template_sections"].as_array().unwrap().last().unwrap();
    assert_eq!(json!(fed::ai_section_lines()), last["lines"]);
    // template = sections joined by "\n" + "\n"
    let joined: Vec<&str> = fed::template_sections()
        .iter()
        .flat_map(|s| s.lines.iter().copied())
        .collect();
    assert_eq!(format!("{}\n", joined.join("\n")), fed::template());
}

fn check_modes(v: &TestVault, base_rel: &str, want: &J) -> Result<(), String> {
    for (rel, mode) in want.as_object().into_iter().flatten() {
        let name = rel.rsplit('/').next().unwrap();
        if name != FED && name != LOCK {
            continue; // umask-dependent entries the original did not create
        }
        let p = v.notes.path().join(rel);
        if p.is_symlink() || p.is_dir() {
            continue;
        }
        let got = format!(
            "{:o}",
            std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&p).unwrap().permissions())
                & 0o777
        );
        eq(&format!("mode {base_rel}{rel}"), &json!(got), mode)?;
    }
    Ok(())
}

#[test]
fn golden_config_template_creation() {
    let g = golden("config_template");
    assert_cases(
        "config_template creation",
        g.cases_for("Config.new (template creation)"),
        |c| {
            let i = &c.input;
            if i["base_path"] == json!("") {
                return Ok(()); // cwd-relative empty NOTES_PATH: not supported natively (see module docs)
            }
            let mut v = TestVault::from_spec(&i["tree"]);
            let tmp = v.notes.path().to_string_lossy().into_owned();
            let env = env_json(&i["env"], &tmp);
            let base = match (i["base_path"].as_str(), i["notes_path_env"].as_str()) {
                (Some(b), _) => b.replace("<ROOT>", &tmp),
                (None, Some(n)) => {
                    let env_with = frankmd_config::EnvSnapshot::from_pairs([(
                        "NOTES_PATH",
                        n.replace("<ROOT>", &tmp),
                    )]);
                    dirs::notes_root(None, &env_with)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned()
                }
                (None, None) => unreachable!(),
            };
            v.root = base.into();
            let readonly = i["readonly_root"].as_bool().unwrap_or(false);
            if readonly && !make_readonly(&v.root) {
                return Ok(()); // running as root: permission checks are meaningless
            }
            let store = v.open(env);
            if readonly {
                restore_writable(&v.root);
            }
            let e = &c.expected;
            eq(
                "config_file_path",
                &json!(v.normalize(&store.path().to_string_lossy())),
                &e["config_file_path"],
            )?;
            let content = std::fs::read_to_string(store.path())
                .ok()
                .filter(|_| store.path().is_file());
            eq("fed_content", &json!(content), &e["fed_content"])?;
            eq(
                "fed_equals_template",
                &json!(content.as_deref() == Some(fed::template())),
                &e["fed_equals_template"],
            )?;
            let rc = store.current();
            eq("file_values", &file_values_json(&rc), &e["file_values"])?;
            for (k, want) in e["resolved_sample"].as_object().unwrap() {
                eq(&format!("resolved.{k}"), &get_json(&rc, k), want)?;
            }
            eq(
                "tree_after",
                &v.snapshot(),
                &expand_template(&e["tree_after"]),
            )?;
            check_modes(&v, "", &e["modes"])
        },
    );
}

#[test]
fn golden_config_template_upgrade_pure() {
    let g = golden("config_template");
    assert_cases(
        "config_template upgrade (pure)",
        g.cases_for("upgrade_config_file (via Config.new)"),
        |c| {
            let fed_in = c.input["fed"].as_str().unwrap();
            let changed = c.expected["changed"].as_bool().unwrap();
            eq(
                "needs_ai_section",
                &json!(fed::needs_ai_section(fed_in)),
                &json!(changed),
            )?;
            let want = changed.then(|| c.expected["fed_after"].as_str().unwrap().to_owned());
            eq(
                "upgrade_ai_section",
                &json!(fed::upgrade_ai_section(fed_in)),
                &json!(want),
            )
        },
    );
}

#[test]
fn golden_config_template_upgrade_on_open() {
    let g = golden("config_template");
    assert_cases(
        "config_template upgrade (store)",
        g.cases_for("upgrade_config_file (via Config.new)"),
        |c| {
            let v = TestVault::with_fed(c.input["fed"].as_str().unwrap());
            chmod(
                &v.fed_path(),
                u32::from_str_radix(c.input["fed_mode"].as_str().unwrap(), 8).unwrap(),
            );
            let store = v.open_empty_env();
            eq("fed_after", &json!(v.read_fed()), &c.expected["fed_after"])?;
            eq(
                "fed_mode_after",
                &json!(format!("{:o}", v.mode(FED))),
                &c.expected["fed_mode_after"],
            )?;
            eq(
                "file_values",
                &file_values_json(&store.current()),
                &c.expected["file_values"],
            )
        },
    );
}

// ------------------------------------------------------------------ config_writeback

/// JSON object with key order preserved (serde_json's Map may sort keys,
/// but `update` appends missing keys in hash insertion order).
#[derive(Debug, Default)]
struct Ordered(Vec<(String, J)>);

impl<'de> Deserialize<'de> for Ordered {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = Ordered;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an object")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut m: A,
            ) -> Result<Ordered, A::Error> {
                let mut out = Vec::new();
                while let Some((k, v)) = m.next_entry::<String, J>()? {
                    out.push((k, v));
                }
                Ok(Ordered(out))
            }
        }
        d.deserialize_map(V)
    }
}

#[derive(Debug, Deserialize)]
struct WbOp {
    op: String,
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    value: J,
    #[serde(default)]
    values: Option<Ordered>,
    #[serde(default)]
    fed: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WbInput {
    ops: Vec<WbOp>,
}

#[derive(Debug, Deserialize)]
struct WbCase {
    name: String,
    input: WbInput,
}

#[derive(Debug, Deserialize)]
struct WbFile {
    cases: Vec<WbCase>,
}

fn ordered_ops() -> std::collections::HashMap<String, Vec<WbOp>> {
    let f: WbFile =
        serde_json::from_slice(&frankmd_testkit::golden_bytes("config_writeback.json")).unwrap();
    f.cases.into_iter().map(|c| (c.name, c.input.ops)).collect()
}

/// Expected snapshot after step `i`, taken from a fixture value that reflects
/// disk (see module docs).
fn expected_disk_values(c: &Case, i: usize) -> J {
    let steps = c.expected["steps"].as_array().unwrap();
    let step = &steps[i];
    if step["op"] == json!("external_write") {
        return step["instance_values"].clone(); // nobody re-read the file yet
    }
    match steps.get(i + 1) {
        Some(next) if next["op"] == json!("reload") => {
            next["result"]["reloaded_file_values"].clone()
        }
        None => c.expected["reread_file_values"].clone(),
        _ => step["instance_values"].clone(),
    }
}

/// A raw fixture input as the untyped Ruby object `format_value` receives.
fn uncast(v: &J) -> Option<Value> {
    match v {
        J::Null => None,
        J::String(s) => Some(Value::Str(s.clone())),
        J::Bool(b) => Some(Value::Bool(*b)),
        J::Number(n) => Some(
            n.as_i64()
                .map_or_else(|| Value::Json(v.clone()), |i| Value::Int(i.into())),
        ),
        other => Some(Value::Json(other.clone())),
    }
}

fn run_writeback_case(c: &Case, ops: &[WbOp]) -> Result<(), String> {
    let i = &c.input;
    let v = TestVault::from_spec(&i["tree"]);
    if let Some(fed) = i["fed"].as_str() {
        v.write_fed(fed.as_bytes());
        v.chmod(FED, 0o600);
    }
    let env = env_json(&i["env"], &v.notes.path().to_string_lossy());
    let mut store = v.open(env.clone());
    let readonly = i["readonly_root"].as_bool().unwrap_or(false);
    if readonly && !make_readonly(&v.root) {
        return Ok(());
    }
    let result = (|| {
        let e = &c.expected;
        let after_load = match &e["fed_after_load"] {
            J::String(s) if s == "<UNCHANGED>" => i["fed"].clone(),
            other => expand_template(other),
        };
        eq(
            "fed_after_load",
            &json!(
                v.read_fed_bytes()
                    .map(|b| String::from_utf8_lossy(&b).into_owned())
            ),
            &after_load,
        )?;

        for (n, (op, step)) in ops.iter().zip(e["steps"].as_array().unwrap()).enumerate() {
            let tag = format!("step {n} {}", op.op);
            let want = &step["result"];
            match op.op.as_str() {
                "set" => {
                    let ok = match Key::from_name(op.key.as_deref().unwrap()) {
                        None => false,
                        Some(k) => coerce(k, &op.value)
                            .ok()
                            .is_some_and(|val| store.update(&[(k, val)]).is_ok()),
                    };
                    eq(&tag, &json!(ok), &json!(want == &json!(true)))?;
                }
                "update" => {
                    let mut changes = Vec::new();
                    let mut ok = true;
                    for (name, raw) in &op.values.as_ref().unwrap().0 {
                        if let Some(k) = Key::from_name(name) {
                            match coerce(k, raw) {
                                Ok(val) => changes.push((k, val)),
                                Err(_) => ok = false,
                            }
                        }
                    }
                    let ok = ok && store.update(&changes).is_ok();
                    eq(&tag, &json!(ok), want)?;
                }
                "reload" => {
                    let rc = store.reload().map_err(|e| format!("{tag}: {e}"))?;
                    eq(&tag, &file_values_json(&rc), &want["reloaded_file_values"])?;
                }
                "external_write" => v.write_fed(op.fed.as_deref().unwrap().as_bytes()),
                "modify_append" | "modify_replace" => {
                    let Some(k) = Key::from_name(op.key.as_deref().unwrap()) else {
                        eq(&tag, &json!({"returned": false, "block_saw": null}), want)?;
                        continue;
                    };
                    let mut saw = J::Null;
                    let append = op.op == "modify_append";
                    let item = op.value.clone();
                    let res = store.modify(k, |old| {
                        saw = opt_json(old.as_ref());
                        if append {
                            let mut arr = old
                                .map(|o| o.to_json())
                                .and_then(|j| j.as_array().cloned())
                                .unwrap_or_default();
                            arr.push(item);
                            Some(Value::Json(J::Array(arr)))
                        } else {
                            coerce(k, &item).unwrap()
                        }
                    });
                    eq(
                        &tag,
                        &json!({"returned": res.is_ok(), "block_saw": saw}),
                        want,
                    )?;
                }
                "ui_settings" => {
                    eq(&tag, &J::Object(ui_settings_json(&store.current())), want)?;
                }
                "format_value" => {
                    let k = key(op.key.as_deref().unwrap());
                    // mechanical: the fixture calls the private `format_value` with the
                    // *uncast* input (config.rb:630, `"#{key} = #{value ? 'true' : 'false'}"`
                    // makes `"x"` truthy), so pass it through by JSON kind instead of `coerce`.
                    let got = store
                        .current()
                        .format_line(k, uncast(&op.value).as_ref())
                        .map_or_else(
                            |e| json!({"error": "ArgumentError", "message": e.to_string()}),
                            |l| json!(l),
                        );
                    eq(&tag, &got, want)?;
                }
                "generate_config_lines" => continue, // dead code in the original
                other => return Err(format!("unknown op {other}")),
            }
            let disk = v
                .read_fed_bytes()
                .map(|b| String::from_utf8_lossy(&b).into_owned());
            let disk = disk.filter(|_| v.fed_exists());
            eq(&format!("{tag} fed"), &json!(disk), &step["fed"])?;
            eq(
                &format!("{tag} snapshot"),
                &file_values_json(&store.current()),
                &expected_disk_values(c, n),
            )?;
        }
        Ok(())
    })();
    if readonly {
        restore_writable(&v.root);
    }
    result?;
    let fresh = v.open(env);
    eq(
        "reread_file_values",
        &file_values_json(&fresh.current()),
        &c.expected["reread_file_values"],
    )?;
    eq(
        "tree_after",
        &v.snapshot(),
        &expand_template(&c.expected["tree_after"]),
    )?;
    check_modes(&v, "", &c.expected["modes"])
}

#[test]
fn golden_config_writeback() {
    let g = golden("config_writeback");
    let ops = ordered_ops();
    assert_cases("config_writeback", &g.cases, |c| {
        run_writeback_case(c, &ops[&c.name])
    });
}

//! Golden conformance: `tests/fixtures/golden/tree_helper.json`
//! (`TreeHelper#render_tree_items`).
//!
//! The expected value is Rails HTML. The data part is recovered with a small
//! tag scanner (no regex dev-dependency): every `.tree-item` becomes a row with
//! `data-path`, kind (`data-type` / `data-file-type`), `selected`, chevron
//! `expanded`, `draggable`, `contextmenu`, `data-mtime`, the `.truncate` name,
//! depth (enclosing `tree-children` containers) and visibility (no enclosing
//! `tree-children hidden`). It is compared with [`tree_rows`]. Input items are
//! converted to `TreeNode` leniently, mirroring the helper's tolerance: unknown
//! `type` → file, `children: null` → `[]`, missing `file_type` → markdown,
//! missing `mtime` → 0 (and the row's `mtime` is then expected as `None`, since
//! Rails omitted `data-mtime`). The empty top-level list is `TreeView::Empty`
//! and its localized `sidebar.no_notes_yet` text is checked via frankmd-i18n.
#![allow(missing_docs, clippy::unwrap_used)]

use frankmd_core::tree::TreeNode;
use frankmd_core::tree_view::{RowKind, TreeRow, TreeView, tree_rows};
use frankmd_i18n::{Catalog, Locale};
use frankmd_testkit::serde_json::{self, Value};
use frankmd_testkit::{assert_cases, golden};
use std::collections::BTreeSet;

fn locale(code: &str) -> Locale {
    match code {
        "en" => Locale::En,
        "pt-BR" => Locale::PtBr,
        "pt-PT" => Locale::PtPt,
        "es" => Locale::Es,
        "he" => Locale::He,
        "ja" => Locale::Ja,
        "ko" => Locale::Ko,
        other => panic!("unknown locale {other}"),
    }
}

/// Lenient item → node JSON (see module docs).
fn lenient(item: &Value) -> Value {
    let mut o = item.as_object().unwrap().clone();
    if o.get("type").and_then(Value::as_str) == Some("folder") {
        let kids = o
            .get("children")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        o.insert(
            "children".into(),
            Value::Array(kids.iter().map(lenient).collect()),
        );
    } else {
        o.insert("type".into(), "file".into());
        o.remove("children");
    }
    Value::Object(o)
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&amp;", "&")
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let pat = format!(" {name}=\"");
    let start = tag.find(&pat)? + pat.len();
    let end = tag[start..].find('"')? + start;
    Some(unescape(&tag[start..end]))
}

fn has_class(tag: &str, class: &str) -> bool {
    attr(tag, "class").is_some_and(|c| c.split_whitespace().any(|t| t == class))
}

/// Parse rendered tree HTML back into rows.
fn parse_rows(html: &str) -> Vec<TreeRow> {
    let mut rows: Vec<TreeRow> = Vec::new();
    // Stack of open divs: (is tree-children, hidden).
    let mut stack: Vec<(bool, bool)> = Vec::new();
    let mut i = 0;
    while let Some(off) = html[i..].find('<') {
        let start = i + off;
        let end = start + html[start..].find('>').unwrap();
        let tag = &html[start..=end];
        i = end + 1;
        if tag.starts_with("</div") {
            stack.pop();
        } else if tag.starts_with("<div") {
            let children = has_class(tag, "tree-children");
            stack.push((children, children && has_class(tag, "hidden")));
            if has_class(tag, "tree-item") {
                let depth = stack.iter().filter(|(c, _)| *c).count();
                let visible = !stack.iter().any(|(_, h)| *h);
                let kind = match (
                    attr(tag, "data-type").as_deref(),
                    attr(tag, "data-file-type").as_deref(),
                ) {
                    (Some("folder"), _) => RowKind::Folder,
                    (_, Some("config")) => RowKind::Config,
                    _ => RowKind::Note,
                };
                rows.push(TreeRow {
                    path: attr(tag, "data-path").unwrap(),
                    name: String::new(),
                    depth,
                    kind,
                    expanded: false,
                    selected: has_class(tag, "selected"),
                    draggable: attr(tag, "draggable").as_deref() == Some("true"),
                    context_menu: attr(tag, "data-action")
                        .is_some_and(|a| a.contains("contextmenu")),
                    visible,
                    mtime: attr(tag, "data-mtime").map(|m| m.parse().unwrap()),
                    encrypted: false,
                });
            }
        } else if tag.starts_with("<svg") && has_class(tag, "tree-chevron") {
            if let Some(last) = rows.last_mut() {
                last.expanded = has_class(tag, "expanded");
            }
        } else if tag.starts_with("<span") && has_class(tag, "truncate") {
            let close = i + html[i..].find("</span>").unwrap();
            if let Some(last) = rows.last_mut() {
                last.name = unescape(&html[i..close]);
            }
            i = close;
        }
    }
    rows
}

#[test]
fn golden_render_tree_items() {
    let g = golden("tree_helper");
    assert_cases("tree_helper", g.cases_for("render_tree_items"), |c| {
        let items: Vec<Value> = c.input["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(lenient)
            .collect();
        let nodes: Vec<TreeNode> =
            serde_json::from_value(Value::Array(items)).map_err(|e| e.to_string())?;
        let expanded: BTreeSet<String> = c.input["expanded_folders"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect();
        let selected = c.input["selected_file"].as_str().unwrap();
        let html = c.expected.as_str().unwrap();
        let got = match tree_rows(&nodes, &expanded, selected) {
            // TreeNode has no "mtime absent" state (serde default 0); Rails
            // omitted data-mtime then. Treat a 0 mtime as absent.
            TreeView::Rows(rows) => TreeView::Rows(
                rows.into_iter()
                    .map(|r| TreeRow {
                        mtime: r.mtime.filter(|m| *m != 0),
                        ..r
                    })
                    .collect(),
            ),
            TreeView::Empty => TreeView::Empty,
        };

        if !html.contains("tree-item") {
            // Empty state: the localized "no notes yet" message.
            let cat = Catalog::load(locale(c.input["locale"].as_str().unwrap())).unwrap();
            let text = cat.t("sidebar.no_notes_yet", &[]);
            if !html.contains(&format!(">{text}</div>")) {
                return Err(format!("localized no_notes_yet {text:?} not in {html}"));
            }
            return if got == TreeView::Empty {
                Ok(())
            } else {
                Err(format!("expected Empty, got {got:?}"))
            };
        }
        let want = TreeView::Rows(parse_rows(html));
        if got == want {
            Ok(())
        } else {
            Err(format!("\n    expected {want:?}\n    got      {got:?}"))
        }
    });
}

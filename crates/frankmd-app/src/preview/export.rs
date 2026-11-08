//! Export the rendered note to standalone HTML and plain text.
//!
//! The HTML is generated from the same sanitized IR used by the preview, so
//! raw HTML stays sanitized. Relative image links are resolved by a `<base>`
//! pointing at the note's folder; Chromium can print those local images.

use std::fmt::Write as _;

use frankmd_markdown::ir::{Block, Inline, ListItem};

use super::PreviewPane;

impl PreviewPane {
    /// Standalone HTML for "Copy formatted" and PDF/print.
    pub(crate) fn export_html(&self, title: &str) -> String {
        let body = frankmd_markdown::html::to_html(&self.doc);
        let title = escape(title);
        let base = self
            .link_cx
            .base_dir
            .as_ref()
            .and_then(|dir| url::Url::from_directory_path(dir).ok())
            .map(|url| format!("<base href=\"{}\">", escape(url.as_str())))
            .unwrap_or_default();
        format!(
            "<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title>{base}<style>{CSS}</style></head><body><article class=\"markdown-body\">{body}</article></body></html>"
        )
    }

    /// Plain-text fallback for the rich clipboard payload.
    pub(crate) fn export_plain_text(&self) -> String {
        let mut out = String::new();
        for block in &self.doc.blocks {
            plain_block(block, &mut out);
        }
        out.trim_end().to_owned()
    }
}

const CSS: &str = r#"
:root { color-scheme: light; }
* { box-sizing: border-box; }
html, body { margin: 0; background: #fff; color: #202124; }
body { font: 16px/1.65 -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Noto Sans", sans-serif; }
.markdown-body { max-width: 860px; margin: 0 auto; padding: 40px 44px 64px; }
h1, h2, h3, h4, h5, h6 { line-height: 1.25; margin: 1.4em 0 .55em; }
h1 { font-size: 2em; } h2 { font-size: 1.55em; } h3 { font-size: 1.3em; }
h4, h5, h6 { font-size: 1.1em; }
p { margin: .7em 0; }
a { color: #0b57d0; text-decoration: underline; text-underline-offset: 2px; }
img { max-width: 100%; height: auto; border-radius: 6px; }
blockquote { margin: 1em 0; padding: .2em 0 .2em 1em; border-left: 4px solid #c7c7c7; color: #555; }
pre { overflow-x: auto; padding: 12px 14px; border-radius: 6px; background: #f4f4f5; line-height: 1.45; }
code { font-family: "JetBrains Mono", "Cascadia Code", monospace; font-size: .9em; background: #f1f1f2; padding: .12em .3em; border-radius: 4px; }
pre code { background: none; padding: 0; }
table { width: 100%; border-collapse: collapse; margin: 1em 0; }
th, td { border: 1px solid #ddd; padding: 6px 9px; text-align: left; }
th { background: #f4f4f5; }
hr { border: 0; border-top: 1px solid #ddd; margin: 1.8em 0; }
ul, ol { padding-left: 1.5em; }
li.task-list-item { list-style: none; }
.math-inline, .math-block { font-family: "Latin Modern Math", serif; }
.wikilink { color: #0b57d0; }
@page { size: auto; margin: 16mm 15mm; }
@media print {
  body { background: #fff; }
  .markdown-body { max-width: none; padding: 0; }
  pre, blockquote, img, table { break-inside: avoid; }
  a { color: inherit; }
}
"#;

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn plain_block(block: &Block, out: &mut String) {
    match block {
        Block::Heading { inlines, .. } => {
            plain_inlines(inlines, out);
            out.push_str("\n\n");
        }
        Block::Paragraph { inlines, .. } => {
            plain_inlines(inlines, out);
            out.push('\n');
        }
        Block::Code { lines, .. } => {
            for line in lines.iter() {
                out.push_str(&line.text);
                out.push('\n');
            }
            out.push('\n');
        }
        Block::Math { tex, .. } => {
            let _ = writeln!(out, "{tex}\n");
        }
        Block::Quote { children, .. } => {
            let mut inner = String::new();
            for child in children {
                plain_block(child, &mut inner);
            }
            for line in inner.trim_end().lines() {
                let _ = writeln!(out, "> {line}");
            }
            out.push('\n');
        }
        Block::List { ordered, items, .. } => {
            for (index, item) in items.iter().enumerate() {
                let marker = if ordered.is_some() {
                    format!("{}. ", index + 1)
                } else {
                    "- ".to_owned()
                };
                let mut inner = String::new();
                for child in &item.children {
                    plain_block(child, &mut inner);
                }
                let first = inner.trim_end().lines().next().unwrap_or("");
                let _ = writeln!(out, "{marker}{first}");
                for line in inner.trim_end().lines().skip(1) {
                    let _ = writeln!(out, "  {line}");
                }
                plain_task(item, out);
            }
            out.push('\n');
        }
        Block::Table { header, rows, .. } => {
            for cell in header {
                plain_inlines(cell, out);
                out.push('\t');
            }
            out.push('\n');
            for row in rows {
                for cell in row {
                    plain_inlines(cell, out);
                    out.push('\t');
                }
                out.push('\n');
            }
            out.push('\n');
        }
        Block::Rule { .. } => out.push_str("---\n\n"),
        Block::Embed { .. } => {}
        Block::Html { text, .. } => {
            let _ = writeln!(out, "{text}\n");
        }
    }
}

fn plain_task(item: &ListItem, out: &mut String) {
    if let Some(checked) = item.task {
        let mark = if checked { "[x]" } else { "[ ]" };
        let _ = writeln!(out, "  {mark}");
    }
}

fn plain_inlines(inlines: &[Inline], out: &mut String) {
    for inline in inlines {
        match inline {
            Inline::Text(text) | Inline::Code(text) | Inline::Math(text) | Inline::Html(text) => {
                out.push_str(text)
            }
            Inline::Emph(children)
            | Inline::Strong(children)
            | Inline::Strike(children)
            | Inline::Link { children, .. } => plain_inlines(children, out),
            Inline::Superscript(text)
            | Inline::Subscript(text)
            | Inline::Highlight(text)
            | Inline::FootnoteRef(text) => out.push_str(text),
            Inline::Emoji { emoji, .. } => out.push_str(emoji),
            Inline::Icon { .. } => {}
            Inline::WikiLink { target, label, .. } => {
                out.push_str(label.as_deref().unwrap_or(target));
            }
            Inline::Image { alt, .. } => out.push_str(alt),
            Inline::SoftBreak | Inline::HardBreak => out.push('\n'),
        }
    }
}

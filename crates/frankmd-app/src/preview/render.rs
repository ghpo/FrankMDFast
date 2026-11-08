//! Element building for [`PreviewPane`]: one list row at a time.

use crate::scale::px;
use std::sync::Arc;

use frankmd_markdown::embed::Embed;
use frankmd_markdown::highlight::language_for_fence;
use frankmd_markdown::ir::{Align, Block, HighlightedLine, Inline, ListItem};
use frankmd_markdown::math::MathStyle;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::base::TestTextExt as _;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::{
    AnyElement, Context, ElementId, FontWeight, HighlightStyle, InteractiveElement as _,
    InteractiveText, IntoElement, KeyDownEvent, ObjectFit, ParentElement as _, Pixels, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, StyledImage as _, StyledText,
    Window, div, img, list, svg,
};

use super::PreviewPane;
use super::inline::{self, Builder, LinkRef, Rich, Segment};
use super::items::Part;
use super::links::{self, ImageSource};
use super::remote::RemoteState;
use super::style::Metrics;

/// Accessibility roles we set.
use gpui_kit::accesskit::Role;

impl Render for PreviewPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let m = Metrics::at(self.zoom);
        let viewer = self
            .viewer
            .is_some()
            .then(|| self.render_viewer(window, cx));
        let colors = self.colors.clone();
        let body: AnyElement = if !self.is_markdown {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(colors.muted)
                .child(
                    self.t(
                        "preview.markdown_only",
                        "Preview is available for Markdown notes.",
                    )
                    .test_text(self.t(
                        "preview.markdown_only",
                        "Preview is available for Markdown notes.",
                    )),
                )
                .into_any_element()
        } else {
            list(
                self.list.clone(),
                cx.processor(|this: &mut Self, ix: usize, window, cx| {
                    this.render_row(ix, window, cx)
                }),
            )
            .size_full()
            .into_any_element()
        };
        div()
            .id("preview-pane")
            .key_context("Preview")
            .role(Role::Document)
            .aria_label(self.t("preview.title", "Preview"))
            .test_support()
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .overflow_hidden()
            .bg(colors.background)
            .text_color(colors.text)
            .text_size(m.text)
            .line_height(m.line_height)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, w, cx| this.on_key_down(e, w, cx)))
            .on_mouse_down(
                gpui_kit::MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    if !this.focus.contains_focused(window, cx) {
                        this.focus.focus(window, cx);
                    }
                }),
            )
            .child(body)
            .vertical_scrollbar(&self.list)
            .children(viewer)
    }
}

impl PreviewPane {
    /// Column width in px: `editor_width` ch + 3 rem.
    fn column_width(&self, m: &Metrics) -> Pixels {
        m.text * (0.55 * f32::from(self.width_ch)) + px(48.)
    }

    fn render_row(
        &mut self,
        ix: usize,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(item) = self.layout.items.get(ix).cloned() else {
            return div().into_any_element();
        };
        let m = Metrics::at(self.zoom);
        let doc = self.doc.clone();
        let mut images = item.first_image;
        let content = match &item.part {
            Part::FrontMatter => self.front_matter_row(&m, cx),
            Part::Whole => match doc.blocks.get(item.block) {
                Some(block) => self.block(block, &m, &mut images, item.code_ordinal, ix, cx),
                None => div().into_any_element(),
            },
            Part::CodeLines(range) => match doc.blocks.get(item.block) {
                Some(Block::Code { lang, lines, .. }) => self.code(
                    lang.as_deref(),
                    lines,
                    range.clone(),
                    item.code_ordinal.unwrap_or(0),
                    &m,
                    cx,
                ),
                _ => div().into_any_element(),
            },
            Part::ListItems(range) => match doc.blocks.get(item.block) {
                Some(Block::List { ordered, items, .. }) => {
                    self.list_items(*ordered, items, range.clone(), &m, &mut images, ix, cx)
                }
                _ => div().into_any_element(),
            },
            Part::TableRows(range) => match doc.blocks.get(item.block) {
                Some(Block::Table {
                    align,
                    header,
                    rows,
                    ..
                }) => {
                    let header = (range.start == 0).then_some(header.as_slice());
                    self.table(align, header, &rows[range.clone()], &m, &mut images, ix, cx)
                }
                _ => div().into_any_element(),
            },
        };
        let continues = matches!(&item.part, Part::CodeLines(r) | Part::ListItems(r) | Part::TableRows(r)
            if self.layout.items.get(ix + 1).is_some_and(|n| n.block == item.block && n.part != Part::FrontMatter) && !r.is_empty());
        let width = self.column_width(&m);
        div()
            .w_full()
            .flex()
            .justify_center()
            .px(m.text * 1.5)
            .pt_when(ix == 0, m.text * 1.5)
            .pb(if continues { px(0.) } else { m.gap })
            .child(div().w_full().max_w(width).child(content))
            .into_any_element()
    }

    fn front_matter_row(&mut self, m: &Metrics, cx: &mut Context<Self>) -> AnyElement {
        let open = self.show_front_matter;
        let label = format!(
            "{} {}",
            self.t("preview.front_matter", "Front matter"),
            if open { "▾" } else { "▸" }
        );
        let colors = &self.colors;
        let mut row = div().flex().flex_col().gap(px(4.)).child(
            div()
                .id("preview-front-matter")
                .role(Role::Button)
                .aria_label(label.clone())
                .test_support()
                .text_size(m.code)
                .text_color(colors.faint)
                .cursor_pointer()
                .child(label.clone().test_text(label))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.show_front_matter = !this.show_front_matter;
                    this.list.remeasure();
                    cx.notify();
                })),
        );
        if open && let Some(fm) = self.doc.front_matter.clone() {
            row = row.child(
                div()
                    .bg(colors.code_bg)
                    .rounded(px(4.))
                    .p(px(8.))
                    .font_family(mono_family(cx))
                    .text_size(m.code)
                    .text_color(colors.muted)
                    .child(fm.trim_end().to_owned()),
            );
        }
        row.into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn block(
        &mut self,
        block: &Block,
        m: &Metrics,
        images: &mut usize,
        code_ordinal: Option<usize>,
        row: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = self.colors.clone();
        match block {
            Block::Heading { level, inlines, .. } => {
                let size = m.heading(*level);
                let mut el = div()
                    .text_size(size)
                    .line_height(size * 1.3)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(colors.heading(*level))
                    .pt(m.gap * 0.5)
                    .child(self.flow(inlines, m, images, row, cx));
                if *level <= 2 {
                    el = el.border_b_1().border_color(colors.border).pb(m.gap * 0.4);
                }
                el.into_any_element()
            }
            Block::Paragraph { inlines, .. } => self.flow(inlines, m, images, row, cx),
            Block::Code { lang, lines, .. } => self.code(
                lang.as_deref(),
                lines,
                0..lines.len(),
                code_ordinal.unwrap_or(0),
                m,
                cx,
            ),
            Block::Math { tex, span } => {
                let el = self.math(tex, MathStyle::Display, m, cx);
                div()
                    .id(ElementId::NamedInteger(
                        "preview-math".into(),
                        u64::from(span.start_line),
                    ))
                    .w_full()
                    .flex()
                    .justify_center()
                    .overflow_x_scroll()
                    .py(m.gap * 0.5)
                    .child(el)
                    .into_any_element()
            }
            Block::Quote {
                alert, children, ..
            } => {
                let accent = alert.map(|k| colors.alert(k)).unwrap_or(colors.border);
                let mut col = div()
                    .flex()
                    .flex_col()
                    .gap(m.gap)
                    .border_l_4()
                    .border_color(accent)
                    .pl(m.text)
                    .text_color(colors.text_secondary);
                if let Some(kind) = alert {
                    let title = format!("{kind:?}");
                    col = col.child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(accent)
                            .child(title.clone().test_text(title)),
                    );
                }
                for child in children {
                    col = col.child(self.block(child, m, images, None, row, cx));
                }
                col.into_any_element()
            }
            Block::List { ordered, items, .. } => {
                self.list_items(*ordered, items, 0..items.len(), m, images, row, cx)
            }
            Block::Table {
                align,
                header,
                rows,
                ..
            } => self.table(align, Some(header), rows, m, images, row, cx),
            Block::Rule { .. } => div()
                .w_full()
                .h(px(1.))
                .my(m.gap * 0.5)
                .bg(colors.border)
                .into_any_element(),
            Block::Embed { embed, .. } => self.embed(embed, m, cx),
            Block::Html { text, .. } => {
                // SECURITY: raw HTML is never interpreted; show it as text.
                let text = text.trim_end_matches('\n').to_owned();
                div()
                    .bg(colors.code_bg)
                    .rounded(px(4.))
                    .p(px(8.))
                    .font_family(mono_family(cx))
                    .text_size(m.code)
                    .text_color(colors.muted)
                    .child(text.clone().test_text(text))
                    .into_any_element()
            }
        }
    }

    /// An inline flow: one wrapped text when possible, else word pieces and
    /// inline objects in a wrapping row; images break the flow.
    fn flow(
        &mut self,
        inlines: &[Inline],
        m: &Metrics,
        images: &mut usize,
        row: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut builder = Builder::new(&self.colors, *images);
        builder.push(inlines);
        *images = builder.next_image();
        let segments = builder.finish();
        let mut groups: Vec<AnyElement> = Vec::new();
        let mut run: Vec<Segment> = Vec::new();
        let mut seq = 0usize;
        for seg in segments {
            if let Segment::Image {
                ordinal,
                src,
                alt,
                link,
            } = seg
            {
                if !run.is_empty() {
                    groups.push(self.inline_run(std::mem::take(&mut run), m, row, &mut seq, cx));
                }
                groups.push(self.image(ordinal, src.as_deref(), &alt, link, m, cx));
            } else {
                run.push(seg);
            }
        }
        if !run.is_empty() {
            groups.push(self.inline_run(run, m, row, &mut seq, cx));
        }
        match groups.len() {
            1 => groups.pop().unwrap_or_else(|| div().into_any_element()),
            _ => div()
                .flex()
                .flex_col()
                .gap(m.gap * 0.5)
                .children(groups)
                .into_any_element(),
        }
    }

    fn inline_run(
        &mut self,
        run: Vec<Segment>,
        m: &Metrics,
        row: usize,
        seq: &mut usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if let [Segment::Text(rich)] = run.as_slice() {
            *seq += 1;
            return self.rich_text(rich, row, *seq, cx);
        }
        let mut wrap = div().flex().flex_row().flex_wrap().items_baseline();
        for seg in run {
            match seg {
                Segment::Text(rich) => {
                    for word in inline::split_words(&rich) {
                        *seq += 1;
                        wrap = wrap.child(self.rich_text(&word, row, *seq, cx));
                    }
                }
                Segment::Math(tex) => wrap = wrap.child(self.math(&tex, MathStyle::Inline, m, cx)),
                Segment::Icon(name) => {
                    wrap = wrap.child(
                        svg()
                            .path(format!("icons/ph/{name}.svg"))
                            .size(m.text)
                            .text_color(self.colors.text),
                    )
                }
                Segment::Image { .. } => {}
            }
        }
        wrap.into_any_element()
    }

    fn rich_text(
        &mut self,
        rich: &Rich,
        row: usize,
        seq: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mono = mono_family(cx);
        let text = StyledText::new(rich.text.clone())
            .with_highlights(rich.highlights.clone())
            .with_font_family_overrides(
                rich.mono
                    .iter()
                    .map(|r| (r.clone(), mono.clone()))
                    .collect::<Vec<_>>(),
            );
        let shown = rich.text.clone();
        if rich.links.is_empty() {
            return text.test_text(shown).into_any_element();
        }
        let ranges: Vec<_> = rich.links.iter().map(|(r, _)| r.clone()).collect();
        let targets: Arc<Vec<LinkRef>> =
            Arc::new(rich.links.iter().map(|(_, l)| l.clone()).collect());
        let weak = cx.entity().downgrade();
        let id = ElementId::NamedInteger(format!("preview-text-{row}").into(), seq as u64);
        let link_text = InteractiveText::new(id, text)
            .on_click(ranges, move |ix, window, cx| {
                let Some(target) = targets.get(ix).cloned() else {
                    return;
                };
                weak.update(cx, |this, cx| this.activate_link(&target, window, cx))
                    .ok();
            })
            .test_text(shown);
        // Observable wrapper so headless tests (and a11y) can address the link.
        div()
            .id(SharedString::from(format!("preview-link-{row}-{seq}")))
            .role(Role::Link)
            .test_support()
            .child(link_text)
            .into_any_element()
    }

    fn math(
        &mut self,
        tex: &str,
        style: MathStyle,
        m: &Metrics,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        let em = m.text
            * if style == MathStyle::Display {
                1.21
            } else {
                1.1
            };
        match self.math.get(tex, style, self.colors.text_rgb) {
            Ok((formula, image)) => {
                let label = tex.to_owned();
                img(image)
                    .w(em * formula.width_em)
                    .h(em * (formula.ascent_em + formula.descent_em))
                    .mb(-(em * formula.descent_em))
                    .test_alt_text(label)
                    .into_any_element()
            }
            Err(_) => {
                // KaTeX `throwOnError: false`: the source in the error colour.
                let src = if style == MathStyle::Display {
                    format!("$${tex}$$")
                } else {
                    format!("${tex}$")
                };
                div()
                    .text_color(self.colors.danger)
                    .child(src.clone().test_text(src))
                    .into_any_element()
            }
        }
    }

    fn code(
        &mut self,
        lang: Option<&str>,
        lines: &Arc<Vec<HighlightedLine>>,
        range: std::ops::Range<usize>,
        ordinal: usize,
        m: &Metrics,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = self.colors.clone();
        let first = range.start == 0;
        let last = range.end >= lines.len();
        let mut text = String::new();
        let mut highlights: Vec<(std::ops::Range<usize>, HighlightStyle)> = Vec::new();
        for (i, line) in lines[range.clone()].iter().enumerate() {
            if i > 0 {
                text.push('\n');
            }
            let base = text.len();
            text.push_str(&line.text);
            for (r, tok) in &line.runs {
                if r.end <= line.text.len()
                    && line.text.is_char_boundary(r.start)
                    && line.text.is_char_boundary(r.end)
                {
                    let style = HighlightStyle {
                        color: Some(colors.syntax(tok.class)),
                        font_weight: tok.definition.then_some(FontWeight::BOLD),
                        ..Default::default()
                    };
                    highlights.push((base + r.start..base + r.end, style));
                }
            }
        }
        highlights.sort_by_key(|(r, _)| r.start);
        highlights.dedup_by(|b, a| b.0.start < a.0.end);
        let shown = text.clone();
        let body = div()
            .id(ElementId::NamedInteger(
                "preview-code".into(),
                (ordinal * 100_000 + range.start) as u64,
            ))
            .w_full()
            .overflow_x_scroll()
            .whitespace_nowrap()
            .px(m.text)
            .pb(if last { m.text * 0.8 } else { px(0.) })
            .child(
                StyledText::new(text)
                    .with_highlights(highlights)
                    .test_text(shown),
            );
        let mut block = div()
            .flex()
            .flex_col()
            .bg(colors.code_bg)
            .font_family(mono_family(cx))
            .text_size(m.code)
            .line_height(m.code * 1.5);
        if first {
            block = block
                .rounded_t(px(4.))
                .pt(px(6.))
                .child(self.code_header(lang, lines, ordinal, m, cx));
        }
        if last {
            block = block.rounded_b(px(4.));
        }
        block.child(body).into_any_element()
    }

    fn code_header(
        &mut self,
        lang: Option<&str>,
        lines: &Arc<Vec<HighlightedLine>>,
        ordinal: usize,
        m: &Metrics,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = self.colors.clone();
        let label: Option<String> = lang
            .and_then(language_for_fence)
            .map(|l| l.label.to_owned());
        let copied = self.copied == Some(ordinal);
        let button_text = if copied {
            self.t("preview.copied", "Copied")
        } else {
            self.t("preview.copy_code", "Copy code")
        };
        let lines = lines.clone();
        div()
            .flex()
            .flex_row()
            .justify_between()
            .items_center()
            .px(m.text)
            .pb(px(4.))
            .text_size(m.code * 0.85)
            .text_color(colors.faint)
            .child(div().children(label.map(|l| l.clone().test_text(l))))
            .child(
                div()
                    .id(SharedString::from(format!("preview-code-copy-{ordinal}")))
                    .role(Role::Button)
                    .aria_label(self.t("preview.copy_code", "Copy code"))
                    .test_support()
                    .cursor_pointer()
                    .px(px(6.))
                    .rounded(px(4.))
                    .hover(|s| s.bg(colors.hover))
                    .child(button_text.clone().test_text(button_text))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let code = lines
                            .iter()
                            .map(|l| l.text.as_str())
                            .collect::<Vec<_>>()
                            .join("\n");
                        this.copy_code(ordinal, code, cx);
                    })),
            )
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn list_items(
        &mut self,
        ordered: Option<u64>,
        items: &[ListItem],
        range: std::ops::Range<usize>,
        m: &Metrics,
        images: &mut usize,
        row: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = self.colors.clone();
        let mut col = div().flex().flex_col().gap(m.gap * 0.3);
        for (i, item) in items[range.clone()].iter().enumerate() {
            let n = range.start + i;
            let marker: AnyElement = match (item.task, ordered) {
                (Some(checked), _) => self.checkbox(item.span.start_line, checked, m, cx),
                (None, Some(start)) => {
                    let label = format!("{}.", start + n as u64);
                    div()
                        .text_color(colors.muted)
                        .child(label)
                        .into_any_element()
                }
                (None, None) => div().text_color(colors.muted).child("•").into_any_element(),
            };
            let mut body = div().flex_1().min_w_0().flex().flex_col().gap(m.gap * 0.3);
            for child in &item.children {
                body = body.child(self.block(child, m, images, None, row, cx));
            }
            col = col.child(
                div()
                    .flex()
                    .flex_row()
                    .items_start()
                    .gap(m.text * 0.5)
                    .child(div().min_w(m.text * 1.2).flex().justify_end().child(marker))
                    .child(body),
            );
        }
        col.into_any_element()
    }

    fn checkbox(
        &mut self,
        line: u32,
        checked: bool,
        m: &Metrics,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = self.colors.clone();
        let size = m.text * 0.95;
        div()
            .id(SharedString::from(format!("preview-task-{line}")))
            .role(Role::CheckBox)
            .aria_label(self.t("preview.toggle_task", "Toggle task"))
            .test_support()
            .mt((m.line_height - size) * 0.5)
            .size(size)
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(3.))
            .border_1()
            .border_color(if checked { colors.accent } else { colors.muted })
            .bg_when(checked, colors.accent)
            .text_color(colors.accent_text)
            .text_size(size * 0.8)
            .cursor_pointer()
            .children(checked.then_some("✓"))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_task(line, checked, cx)))
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn table(
        &mut self,
        align: &[Align],
        header: Option<&[Vec<Inline>]>,
        rows: &[Vec<Vec<Inline>>],
        m: &Metrics,
        images: &mut usize,
        row: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = self.colors.clone();
        let mut table = div()
            .flex()
            .flex_col()
            .border_1()
            .border_color(colors.border)
            .rounded(px(4.));
        let cell = |el: AnyElement, a: Align| {
            let d = div()
                .flex_1()
                .min_w_0()
                .px(m.text * 0.6)
                .py(m.text * 0.3)
                .border_r_1()
                .border_color(colors.border);
            match a {
                Align::Center => d.text_center(),
                Align::Right => d.text_right(),
                Align::None | Align::Left => d,
            }
            .child(el)
        };
        if let Some(header) = header {
            let mut tr = div()
                .flex()
                .flex_row()
                .bg(colors.surface)
                .font_weight(FontWeight::SEMIBOLD);
            for (i, c) in header.iter().enumerate() {
                let el = self.flow(c, m, images, row, cx);
                tr = tr.child(cell(el, align.get(i).copied().unwrap_or(Align::None)));
            }
            table = table.child(tr);
        }
        for r in rows {
            let mut tr = div()
                .flex()
                .flex_row()
                .border_t_1()
                .border_color(colors.border);
            for (i, c) in r.iter().enumerate() {
                let el = self.flow(c, m, images, row, cx);
                tr = tr.child(cell(el, align.get(i).copied().unwrap_or(Align::None)));
            }
            table = table.child(tr);
        }
        table.into_any_element()
    }

    fn embed(&mut self, embed: &Embed, m: &Metrics, cx: &mut Context<Self>) -> AnyElement {
        let colors = self.colors.clone();
        let Embed::YouTube { id, start } = embed;
        let watch = match start {
            Some(s) => format!("https://www.youtube.com/watch?v={id}&t={s}"),
            None => format!("https://www.youtube.com/watch?v={id}"),
        };
        let thumb_url = format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg");
        let thumb = self.remote_image(&thumb_url, cx);
        let title = self.t("preview.youtube_video", "YouTube video");
        let open = self.t("preview.open_in_browser", "Open in browser");
        let watch_click = watch.clone();
        div()
            .id(SharedString::from(format!("preview-embed-{id}")))
            .role(Role::Link)
            .aria_label(title.clone())
            .test_support()
            .w_full()
            .flex()
            .flex_col()
            .rounded(px(4.))
            .overflow_hidden()
            .border_1()
            .border_color(colors.border)
            .cursor_pointer()
            .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&watch_click)))
            .child(
                div()
                    .w_full()
                    .aspect_ratio(16. / 9.)
                    .relative()
                    .bg(colors.code_bg)
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(
                        thumb.map(|t| img(t).absolute().size_full().object_fit(ObjectFit::Cover)),
                    )
                    .child(
                        div()
                            .size(m.text * 3.)
                            .rounded_full()
                            .bg(colors.background.opacity(0.8))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(colors.text)
                            .child("▶"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_between()
                    .items_center()
                    .px(m.text * 0.8)
                    .py(m.text * 0.4)
                    .bg(colors.surface)
                    .child(title.clone().test_text(title))
                    .child(
                        div()
                            .text_color(colors.accent)
                            .child(open.clone().test_text(open)),
                    ),
            )
            .into_any_element()
    }

    /// Remote image bytes when ready (starts the fetch on first sight).
    fn remote_image(&mut self, url: &str, cx: &mut Context<Self>) -> Option<Arc<gpui_kit::Image>> {
        match self.remote.get(url) {
            Some(RemoteState::Ready(image)) => Some(image),
            Some(_) => None,
            None => {
                self.fetch_remote(url, cx);
                None
            }
        }
    }

    /// Image source for `img()`, or `None` while loading / when blocked.
    pub(super) fn image_source(
        &mut self,
        src: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Option<gpui_kit::ImageSource> {
        match links::classify_image(src?, &self.link_cx) {
            ImageSource::Local(path) => self.decrypted_image_path(&path).map(Into::into),
            ImageSource::Remote(url) => self.remote_image(&url, cx).map(Into::into),
            ImageSource::Blocked => None,
        }
    }

    fn image(
        &mut self,
        ordinal: usize,
        src: Option<&str>,
        alt: &str,
        link: Option<LinkRef>,
        m: &Metrics,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = self.colors.clone();
        let focus = self.image_focus(ordinal, cx);
        let source = self.image_source(src, cx);
        let placeholder_text = if alt.is_empty() {
            self.t("preview.image_viewer.image", "image").to_string()
        } else {
            alt.to_owned()
        };
        let placeholder = {
            let colors = colors.clone();
            let text = placeholder_text.clone();
            move || {
                div()
                    .px(px(8.))
                    .py(px(4.))
                    .rounded(px(4.))
                    .bg(colors.code_bg)
                    .text_color(colors.muted)
                    .child(format!("🖼 {text}"))
                    .into_any_element()
            }
        };
        let picture: AnyElement = match source {
            Some(source) => {
                let fallback = placeholder.clone();
                img(source)
                    .max_w_full()
                    .rounded(px(4.))
                    .object_fit(ObjectFit::Contain)
                    .with_fallback(fallback)
                    .test_alt_text(alt.to_owned())
                    .into_any_element()
            }
            None => placeholder(),
        };
        let open_label = self
            .catalog
            .t("preview.image_viewer.open", &[("name", &placeholder_text)]);
        let link_for_click = link.clone();
        let mut el = div()
            .id(SharedString::from(format!("preview-image-{ordinal}")))
            .role(if link.is_some() {
                Role::Link
            } else {
                Role::Image
            })
            .aria_label(if alt.is_empty() {
                placeholder_text.clone()
            } else {
                alt.to_owned()
            })
            .test_support()
            .track_focus(&focus)
            .relative()
            .max_w_full()
            .rounded(px(4.))
            .focus_visible(|s| s.border_2().border_color(colors.accent))
            .cursor_pointer()
            .on_click(
                cx.listener(move |this, _, window, cx| match &link_for_click {
                    Some(link) => this.activate_link(link, window, cx),
                    None => this.open_viewer(ordinal, window, cx),
                }),
            )
            .on_key_down(cx.listener(move |this, e: &KeyDownEvent, window, cx| {
                if matches!(e.keystroke.key.as_str(), "enter" | "space")
                    && !e.keystroke.modifiers.modified()
                {
                    this.open_viewer(ordinal, window, cx);
                    cx.stop_propagation();
                }
            }))
            .child(picture);
        if link.is_some() {
            // OQ-25: a linked image follows its link; enlarging is a separate control.
            el = el.child(
                div()
                    .id(SharedString::from(format!(
                        "preview-image-enlarge-{ordinal}"
                    )))
                    .role(Role::Button)
                    .aria_label(open_label)
                    .test_support()
                    .absolute()
                    .top(px(4.))
                    .right(px(4.))
                    .px(px(6.))
                    .rounded(px(4.))
                    .bg(colors.background.opacity(0.8))
                    .text_size(m.code)
                    .child("⤢")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.open_viewer(ordinal, window, cx);
                    })),
            );
        }
        el.into_any_element()
    }
}

/// The monospace family from the gpui-kit theme (resolved via fontconfig by the shell).
fn mono_family(cx: &gpui_kit::App) -> SharedString {
    gpui_kit::component::ActiveTheme::theme(cx)
        .mono_font_family
        .clone()
}

/// Small conditional-style helpers that keep the builders flat.
trait StyleExt: gpui_kit::Styled + Sized {
    fn pt_when(self, first: bool, pad: Pixels) -> Self {
        if first { self.pt(pad) } else { self }
    }
    fn bg_when(self, on: bool, color: gpui_kit::Hsla) -> Self {
        if on { self.bg(color) } else { self }
    }
}
impl<T: gpui_kit::Styled + Sized> StyleExt for T {}

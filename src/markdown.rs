//! Display-only CommonMark pipeline. No application, persistence or protocol state.
//!
//! `parse` retains semantic blocks and inline styles; `render` lays them out in
//! terminal cells. Task markers are document text, never verification results.
//! At width zero no cells are emitted; an oversized grapheme becomes `?`.
//! Links/images are text only, and HTML is never interpreted.
//!
//! Human-started verification (not executed by the implementation agent):
//! regenerate Cargo.lock with `cargo generate-lockfile`, then run
//! `cargo test --lib markdown::tests` and `cargo test --lib ui::markdown_tests`.
//! For manual inspection, pass `render` output to `ui::style_markdown` with
//! both light values, widths 1/12/80, and mixed CJK/emoji, nested lists,
//! incomplete fences, HTML, long URLs and wide tables. Inspect role labels
//! supplied by the caller separately from document checkboxes; resizing should
//! re-layout the retained Document, never mutate source or application state.
use crate::ui::safe_text;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Role {
    #[default]
    Body,
    User,
    Assistant,
    System,
    Field,
    Constraint,
    Acceptance,
}

/// Real application status is distinct from Markdown's literal `[x]` markers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    None,
    Pending,
    Success,
    Failure,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Context {
    pub role: Role,
    pub status: Status,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InlineStyle {
    pub heading: u8,
    pub strong: bool,
    pub emphasis: bool,
    pub strike: bool,
    pub code: bool,
    pub link: bool,
    pub quote: bool,
    pub marker: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fragment {
    pub text: String,
    pub style: InlineStyle,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SemanticLine {
    pub fragments: Vec<Fragment>,
    pub context: Context,
}
impl SemanticLine {
    pub fn text(&self) -> String {
        self.fragments.iter().map(|s| s.text.as_str()).collect()
    }
    pub fn width(&self) -> usize {
        self.text().width()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Line(Vec<Fragment>),
    /// Separating space inserted by the renderer, not a literal empty code line.
    Spacing,
    Rule,
    Table { headers: Vec<Vec<Fragment>>, rows: Vec<Vec<Vec<Fragment>>> },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Document {
    pub blocks: Vec<Block>,
}

#[derive(Default)]
struct TableBuilder {
    headers: Vec<Vec<Fragment>>,
    rows: Vec<Vec<Vec<Fragment>>>,
    row: Vec<Vec<Fragment>>,
    cell: Vec<Fragment>,
}
struct ListLevel {
    next: Option<u64>,
    first: bool,
}
#[derive(Default)]
struct Builder {
    document: Document,
    line: Vec<Fragment>,
    style: InlineStyle,
    inline_stack: Vec<InlineStyle>,
    links: Vec<String>,
    lists: Vec<ListLevel>,
    quotes: usize,
    table: Option<TableBuilder>,
}

fn append(spans: &mut Vec<Fragment>, text: &str, style: InlineStyle) {
    let text = safe_text(text);
    if text.is_empty() { return; }
    if let Some(last) = spans.last_mut().filter(|s| s.style == style) {
        last.text.push_str(&text);
    } else {
        spans.push(Fragment { text, style });
    }
}
impl Builder {
    fn prefix(&mut self) {
        if !self.line.is_empty() { return; }
        let marker = InlineStyle { marker: true, quote: self.quotes > 0, ..Default::default() };
        append(&mut self.line, &"> ".repeat(self.quotes), marker);
        for (i, list) in self.lists.iter_mut().enumerate() {
            if i > 0 { append(&mut self.line, "  ", marker); }
            if list.first {
                let text = match list.next {
                    Some(n) => { list.next = Some(n.saturating_add(1)); format!("{n}. ") }
                    None => "• ".to_owned(),
                };
                append(&mut self.line, &text, marker);
                list.first = false;
            } else {
                append(&mut self.line, "  ", marker);
            }
        }
    }
    fn push(&mut self, text: &str, mut style: InlineStyle) {
        style.quote |= self.quotes > 0;
        if let Some(table) = &mut self.table {
            // Table cells cannot create terminal rows outside the table layout.
            append(&mut table.cell, &text.replace('\n', " "), style);
            return;
        }
        let safe = safe_text(text);
        for (i, part) in safe.split('\n').enumerate() {
            if i > 0 { self.flush(true); }
            if !part.is_empty() {
                self.prefix();
                append(&mut self.line, part, style);
            }
        }
    }
    fn flush(&mut self, empty: bool) {
        if !self.line.is_empty() || empty {
            self.document.blocks.push(Block::Line(std::mem::take(&mut self.line)));
        }
    }
    fn blank(&mut self) {
        self.flush(false);
        if !matches!(self.document.blocks.last(), Some(Block::Spacing)) {
            self.document.blocks.push(Block::Spacing);
        }
    }
    fn inline(&mut self, change: impl FnOnce(&mut InlineStyle)) {
        self.inline_stack.push(self.style);
        change(&mut self.style);
    }
    fn end_inline(&mut self) {
        self.style = self.inline_stack.pop().unwrap_or_default();
    }
}

/// Parse the complete source (including incomplete streamed Markdown) once.
pub fn parse(source: &str) -> Document {
    // Treat CRLF as a source line ending, not as a terminal control. Lone CR
    // remains protected by safe_text. This display copy never rewrites source.
    let source = safe_text(&source.replace("\r\n", "\n"));
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut b = Builder::default();
    for event in Parser::new_ext(&source, options) {
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => b.flush(false),
                Tag::Heading { level, .. } => {
                    b.flush(false);
                    b.inline(|s| s.heading = level as u8);
                    b.push(&format!("{} ", "#".repeat(level as usize)), b.style);
                }
                Tag::BlockQuote(_) => { b.flush(false); b.quotes += 1; }
                Tag::CodeBlock(_) => {
                    b.flush(false);
                    b.inline(|s| s.code = true);
                }
                Tag::List(start) => {
                    b.flush(false);
                    b.lists.push(ListLevel { next: start, first: false });
                }
                Tag::Item => {
                    b.flush(false);
                    if let Some(list) = b.lists.last_mut() { list.first = true; }
                }
                Tag::Emphasis => b.inline(|s| s.emphasis = true),
                Tag::Strong => b.inline(|s| s.strong = true),
                Tag::Strikethrough => b.inline(|s| s.strike = true),
                Tag::Link { dest_url, .. } => {
                    b.inline(|s| s.link = true);
                    b.links.push(dest_url.to_string());
                }
                Tag::Image { dest_url, .. } => {
                    b.inline(|s| s.link = true);
                    b.links.push(dest_url.to_string());
                    b.push("[图片: ", b.style);
                }
                Tag::Table(_) => { b.flush(false); b.table = Some(TableBuilder::default()); }
                Tag::TableHead => {}
                Tag::TableRow => {}
                Tag::TableCell => {
                    if let Some(t) = &mut b.table { t.cell.clear(); }
                }
                // Disabled extensions have no generated events. If later enabled,
                // their textual children still pass through without being hidden.
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph => {
                    b.flush(false);
                    if b.lists.is_empty() && b.quotes == 0 { b.blank(); }
                }
                TagEnd::Heading(_) => { b.flush(false); b.end_inline(); b.blank(); }
                TagEnd::BlockQuote(_) => { b.flush(false); b.quotes = b.quotes.saturating_sub(1); }
                TagEnd::CodeBlock => { b.flush(false); b.end_inline(); b.blank(); }
                TagEnd::Item => {
                    if b.lists.last().is_some_and(|l| l.first) { b.prefix(); }
                    b.flush(false);
                }
                TagEnd::List(_) => { b.flush(false); b.lists.pop(); }
                TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => b.end_inline(),
                TagEnd::Link | TagEnd::Image => {
                    if tag == TagEnd::Image { b.push("]", b.style); }
                    let target = b.links.pop().unwrap_or_default();
                    b.push(&format!(" ({target})"), b.style);
                    b.end_inline();
                }
                TagEnd::TableCell => {
                    if let Some(t) = &mut b.table { t.row.push(std::mem::take(&mut t.cell)); }
                }
                TagEnd::TableHead => {
                    if let Some(t) = &mut b.table {
                        t.headers = std::mem::take(&mut t.row);
                    }
                }
                TagEnd::TableRow => {
                    if let Some(t) = &mut b.table { t.rows.push(std::mem::take(&mut t.row)); }
                }
                TagEnd::Table => {
                    if let Some(t) = b.table.take() {
                        b.document.blocks.push(Block::Table { headers: t.headers, rows: t.rows });
                    }
                    b.blank();
                }
                _ => {}
            },
            Event::Text(text) => b.push(&text, b.style),
            Event::Code(text) => b.push(&text, InlineStyle { code: true, ..b.style }),
            Event::Html(text) | Event::InlineHtml(text) => b.push(&text, b.style),
            Event::SoftBreak => b.push(" ", b.style),
            Event::HardBreak => {
                if b.table.is_some() { b.push(" ", b.style); } else { b.flush(true); }
            }
            Event::Rule => { b.flush(false); b.document.blocks.push(Block::Rule); }
            Event::TaskListMarker(done) => b.push(
                if done { "[x] " } else { "[ ] " },
                InlineStyle { marker: true, ..b.style },
            ),
            Event::FootnoteReference(text) => b.push(&format!("[^{text}]"), b.style),
            Event::InlineMath(text) => b.push(&format!("${text}$"), b.style),
            Event::DisplayMath(text) => b.push(&format!("$${text}$$"), b.style),
        }
    }
    b.flush(false);
    // Paragraph spacing is useful between blocks, not after the whole document.
    while matches!(b.document.blocks.last(), Some(Block::Spacing)) {
        b.document.blocks.pop();
    }
    b.document
}

/// Wrap across fragment boundaries, not within independently segmented spans.
/// A combining sequence split by emphasis still forms a single grapheme; the
/// style of its first scalar wins. All output is sanitized again here.
pub fn wrap(fragments: &[Fragment], width: usize, context: Context) -> Vec<SemanticLine> {
    if width == 0 { return Vec::new(); }
    let mut text = String::new();
    let mut ranges = Vec::new();
    for fragment in fragments {
        let start = text.len();
        text.push_str(&safe_text(&fragment.text));
        ranges.push((start, text.len(), fragment.style));
    }
    let mut result = Vec::new();
    let mut line = SemanticLine { context, ..Default::default() };
    let mut cells = 0;
    let mut range = 0;
    for (index, glyph) in text.grapheme_indices(true) {
        while range + 1 < ranges.len() && index >= ranges[range].1 { range += 1; }
        let style = ranges.get(range).map_or(InlineStyle::default(), |r| r.2);
        if glyph == "\n" {
            result.push(std::mem::replace(&mut line, SemanticLine { context, ..Default::default() }));
            cells = 0;
            continue;
        }
        let size = glyph.width();
        let (glyph, size) = if size > width { ("?", 1) } else { (glyph, size) };
        if cells + size > width {
            result.push(std::mem::replace(&mut line, SemanticLine { context, ..Default::default() }));
            cells = 0;
        }
        append(&mut line.fragments, glyph, style);
        cells += size;
    }
    result.push(line);
    result
}

fn cell_width(cell: &[Fragment]) -> usize {
    cell.iter().map(|f| f.text.as_str()).collect::<String>().width()
}
fn marker(text: String) -> Fragment {
    Fragment { text, style: InlineStyle { marker: true, ..Default::default() } }
}

impl Document {
    pub fn layout(&self, width: usize, context: Context) -> Vec<SemanticLine> {
        if width == 0 { return Vec::new(); }
        let mut lines = Vec::new();
        for block in &self.blocks {
            match block {
                Block::Line(spans) => lines.extend(wrap(spans, width, context)),
                Block::Spacing => lines.extend(wrap(&[], width, context)),
                Block::Rule => lines.extend(wrap(&[marker("─".repeat(width))], width, context)),
                Block::Table { headers, rows } => {
                    let count = rows.iter().map(Vec::len).chain([headers.len()]).max().unwrap_or(0);
                    let mut widths = vec![1; count];
                    for row in std::iter::once(headers).chain(rows.iter()) {
                        for (i, cell) in row.iter().enumerate() {
                            widths[i] = widths[i].max(cell_width(cell));
                        }
                    }
                    let needed = widths.iter().sum::<usize>().saturating_add(count.saturating_sub(1) * 3);
                    if needed <= width {
                        for (index, row) in std::iter::once(headers).chain(rows.iter()).enumerate() {
                            let mut spans = Vec::new();
                            for (i, size) in widths.iter().enumerate() {
                                if i > 0 { spans.push(marker(" │ ".to_owned())); }
                                let cell = row.get(i).cloned().unwrap_or_default();
                                for mut span in cell.iter().cloned() {
                                    if index == 0 { span.style.strong = true; }
                                    spans.push(span);
                                }
                                spans.push(marker(" ".repeat(size.saturating_sub(cell_width(&cell)))));
                            }
                            lines.extend(wrap(&spans, width, context));
                            if index == 0 {
                                let separator = widths.iter().map(|w| "─".repeat(*w)).collect::<Vec<_>>().join("─┼─");
                                lines.extend(wrap(&[marker(separator)], width, context));
                            }
                        }
                    } else {
                        // Column-name/value pairs retain every cell on narrow screens.
                        if rows.is_empty() {
                            for header in headers {
                                let mut spans = header.clone();
                                for span in &mut spans { span.style.strong = true; }
                                lines.extend(wrap(&spans, width, context));
                            }
                            continue;
                        }
                        for (r, row) in rows.iter().enumerate() {
                            if r > 0 { lines.extend(wrap(&[], width, context)); }
                            for i in 0..count {
                                let mut spans = headers.get(i).cloned().unwrap_or_default();
                                if spans.is_empty() { spans.push(marker(format!("列 {}", i + 1))); }
                                for span in &mut spans { span.style.strong = true; }
                                spans.push(marker(": ".to_owned()));
                                spans.extend(row.get(i).cloned().unwrap_or_default());
                                lines.extend(wrap(&spans, width, context));
                            }
                        }
                    }
                }
            }
        }
        lines
    }
}

pub fn render(source: &str, width: usize, context: Context) -> Vec<SemanticLine> {
    parse(source).layout(width, context)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(source: &str, width: usize) -> String {
        render(source, width, Context::default()).iter().map(SemanticLine::text).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn blocks_inline_styles_and_document_checkboxes() {
        let source = "# Title\n\n**bold** *italic* ~~gone~~ `literal*`\n\n> quote\n\n1. outer\n   - inner\n\n- [x] source marker\n\n---\n\n```rust\n*a*\n```\n\n    indented\n\n[link](https://example.com) ![alt](image.png)";
        let lines = render(source, 100, Context::default());
        let spans: Vec<_> = lines.iter().flat_map(|l| &l.fragments).collect();
        assert!(spans.iter().any(|s| s.style.heading == 1));
        assert!(spans.iter().any(|s| s.style.strong));
        assert!(spans.iter().any(|s| s.style.emphasis));
        assert!(spans.iter().any(|s| s.style.strike));
        assert!(spans.iter().any(|s| s.style.code && s.text.contains("*a*")));
        let output = text(source, 100);
        for expected in ["> quote", "1. outer", "• inner", "[x] source marker", "indented", "https://example.com", "[图片: alt] (image.png)"] {
            assert!(output.contains(expected), "missing {expected}: {output}");
        }
        assert!(lines.iter().all(|l| l.context.status == Status::None));
    }

    #[test]
    fn incomplete_source_html_and_controls_are_readable() {
        for source in ["```\nliteral *text", "unclosed **text", "<b>HTML</b>", "a\x1b[31mb\r\u{009b}"] {
            let output = text(source, 80);
            assert!(!output.is_empty());
            assert!(output.chars().all(|c| c == '\n' || !c.is_control()));
        }
        assert!(text("<b>HTML</b>", 80).contains("<b>HTML</b>"));
        assert!(text("```\nliteral *text", 80).contains("literal *text"));
        assert!(text("unclosed **text", 80).contains("**text"));
        assert_eq!(text("plain\r\ntext", 80), "plain text");
    }

    #[test]
    fn unicode_and_links_fit_every_width() {
        let source = "中文 e\u{301} 👩‍💻 [long](https://example.com/a-very-long-target)";
        for width in 1..40 {
            let lines = render(source, width, Context::default());
            assert!(lines.iter().all(|l| l.width() <= width));
        }
        assert!(render(source, 0, Context::default()).is_empty());
        assert_eq!(text("中", 1), "?");
        let output = text("👩‍💻e\u{301}", 2);
        assert_eq!(output, "👩‍💻\ne\u{301}");
    }

    #[test]
    fn grapheme_can_cross_style_boundaries() {
        let fragments = vec![marker("e".into()), Fragment { text: "\u{301}中".into(), style: InlineStyle::default() }];
        let lines = wrap(&fragments, 2, Context::default());
        assert_eq!(lines[0].text(), "e\u{301}");
        assert_eq!(lines[1].text(), "中");
    }

    #[test]
    fn decoded_entities_and_external_fragments_are_sanitized_again() {
        let rendered = text("&#27;[31m &#9; tab [target](https://example.com)", 100);
        assert!(rendered.contains('�'));
        assert!(rendered.chars().all(|c| c == '\n' || !c.is_control()));
        let hostile = [marker("x\x1b[2J\t\u{009b}".into())];
        let wrapped = wrap(&hostile, 4, Context::default());
        assert!(wrapped.iter().all(|l| l.width() <= 4));
        assert!(wrapped.iter().flat_map(|l| l.text().chars().collect::<Vec<_>>()).all(|c| !c.is_control()));
    }

    #[test]
    fn layout_preserves_source_and_context_and_literal_code_blank_lines() {
        let source = String::from("```\none\n\n```\n");
        let document = parse(&source);
        let context = Context { role: Role::Assistant, status: Status::Pending };
        let wide = document.layout(80, context);
        assert_eq!(wide.iter().map(SemanticLine::text).collect::<Vec<_>>(), ["one", ""]);
        let narrow = document.layout(1, context);
        assert!(narrow.iter().all(|l| l.context == context));
        assert_eq!(source, "```\none\n\n```\n");
        assert_eq!(document.layout(80, context), wide);
    }

    #[test]
    fn tables_switch_to_named_columns_without_losing_values() {
        let source = "| Name | Value |\n| --- | --- |\n| 中文 | **long value** |\n| second | `code` |";
        assert!(text(source, 80).contains(" │ "));
        let narrow = text(source, 12);
        assert!(narrow.contains("Name: 中文"));
        assert!(narrow.contains("Value: long"));
        for width in 1..40 {
            assert!(render(source, width, Context::default()).iter().all(|l| l.width() <= width));
        }
    }
}

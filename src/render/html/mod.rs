//! HTML serializer for the document model: emits a fragment of block-level
//! elements, or a complete document wrapped with a base stylesheet.

mod escape;
mod inline;
mod table;

use crate::model::{Block, Document, List, ListItem, MarkerKind, Note, TableKind};
use crate::render::anchors::{AnchorMap, resolve_anchors};
use crate::render::notes::{NoteNumbers, number_notes};
use escape::escape_attr;
use inline::render_inlines;
use std::collections::HashSet;

/// Immutable render context threaded through every render function.
pub(crate) struct Ctx<'a> {
    pub nums: &'a NoteNumbers,
    pub anchors: &'a AnchorMap,
}

/// Convert a document to HTML. `full` wraps the fragment in a complete HTML
/// document with a base stylesheet; `false` returns the bare block fragment.
pub fn document_to_html(doc: &Document, full: bool) -> String {
    let nums = number_notes(doc);
    let anchors = resolve_anchors(doc);
    let rc = Ctx { nums: &nums, anchors: &anchors };

    let body = render_blocks(&doc.blocks, &rc);
    let footnotes = render_footnotes(doc, &rc);
    let fragment = match (body.is_empty(), footnotes.is_empty()) {
        (true, true) => String::new(),
        (false, true) => body,
        (true, false) => footnotes,
        (false, false) => format!("{body}\n{footnotes}"),
    };

    if full {
        wrap_document(&fragment)
    } else {
        fragment
    }
}

fn render_footnotes(doc: &Document, rc: &Ctx) -> String {
    let mut rendered_defs: HashSet<usize> = HashSet::new();
    let mut ordered: Vec<(&Note, usize)> =
        doc.notes.iter().filter_map(|n| rc.nums.get(&n.id).map(|&num| (n, num))).collect();
    ordered.sort_by_key(|(_, num)| *num);

    let mut items: Vec<String> = Vec::new();
    for (note, num) in ordered {
        let body = render_blocks(&note.blocks, rc);
        if body.is_empty() || !rendered_defs.insert(num) {
            continue;
        }
        items.push(format!(
            "<li id=\"fn-{num}\">{body} <a href=\"#fnref-{num}\" class=\"footnote-backref\">↩</a></li>"
        ));
    }
    if items.is_empty() {
        String::new()
    } else {
        format!("<section class=\"footnotes\">\n<ol>\n{}\n</ol>\n</section>", items.join("\n"))
    }
}

fn render_blocks(blocks: &[Block], rc: &Ctx) -> String {
    let parts: Vec<String> = blocks.iter().filter_map(|b| render_block(b, rc)).collect();
    parts.join("\n")
}

pub(crate) fn render_block(block: &Block, rc: &Ctx) -> Option<String> {
    match block {
        Block::Heading { level, content, .. } => {
            let text = render_inlines(content, rc);
            let text = text.trim();
            if text.is_empty() {
                return None;
            }
            let level = (*level).clamp(1, 6);
            Some(format!("<h{level}>{text}</h{level}>"))
        }
        Block::Paragraph(inlines) => {
            let text = render_inlines(inlines, rc);
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(format!("<p>{trimmed}</p>"))
            }
        }
        Block::List(list) => render_list(list, rc),
        // Trivial layout tables are scaffolding; render their content directly.
        Block::Table(t) if t.kind == TableKind::Layout && t.is_single_cell() => {
            let crate::model::CellSlot::Origin(cell) = &t.grid[0][0] else { unreachable!() };
            let inner = render_blocks(&cell.blocks, rc);
            if inner.is_empty() { None } else { Some(inner) }
        }
        Block::Table(t) => {
            let html = table::render_table(t, rc);
            if html.is_empty() { None } else { Some(html) }
        }
        Block::BlockQuote(blocks) => {
            let inner = render_blocks(blocks, rc);
            if inner.is_empty() {
                None
            } else {
                Some(format!("<blockquote>\n{inner}\n</blockquote>"))
            }
        }
        Block::CodeBlock { lang, text } => {
            let lang_attr = lang
                .as_deref()
                .map(|l| format!(" class=\"language-{}\"", escape_attr(l)))
                .unwrap_or_default();
            let body = escape::escape_text(text.trim_end_matches('\n'));
            Some(format!("<pre><code{lang_attr}>{body}</code></pre>"))
        }
        Block::Rule => Some("<hr>".to_string()),
    }
}

fn render_list(list: &List, rc: &Ctx) -> Option<String> {
    if list.items.is_empty() {
        return None;
    }

    // `<ol type>` reproduces simple markers natively. A composite/overridden
    // marker label (which no `marker` + position can reproduce) forces a
    // `<ul>` with a literal marker prefix, matching the Markdown renderer.
    let has_literal_marker = list.items.iter().any(|i| i.marker_label.is_some());
    let use_ol = list.ordered() && !has_literal_marker;

    let mut items = String::new();
    for (i, item) in list.items.iter().enumerate() {
        let content = render_list_item(item, rc);

        let mut prefix = String::new();
        if !use_ol {
            let label = match &item.marker_label {
                Some(label) => label.clone(),
                None if list.marker == MarkerKind::Bullet => String::new(),
                None => list.marker.label(list.start.saturating_add(i as u64)),
            };
            if !label.is_empty() {
                prefix = format!(
                    "<span class=\"marker\">{}</span> ",
                    escape::escape_text(&label)
                );
            }
        }
        let checkbox = match item.checked {
            Some(true) => "<input type=\"checkbox\" disabled checked> ",
            Some(false) => "<input type=\"checkbox\" disabled> ",
            None => "",
        };
        items.push_str(&format!("<li>{prefix}{checkbox}{content}</li>\n"));
    }

    if use_ol {
        Some(format!(
            "<ol type=\"{}\" start=\"{}\">\n{items}</ol>",
            marker_type(list.marker),
            list.start
        ))
    } else {
        Some(format!("<ul>\n{items}</ul>"))
    }
}

/// Render an item's blocks. The first paragraph stays inline (no `<p>`); any
/// following blocks — nested lists included — render as full block elements.
fn render_list_item(item: &ListItem, rc: &Ctx) -> String {
    let mut parts: Vec<String> = Vec::new();
    for (i, block) in item.blocks.iter().enumerate() {
        if i == 0
            && let Block::Paragraph(inlines) = block
        {
            parts.push(render_inlines(inlines, rc));
            continue;
        }
        if let Some(html) = render_block(block, rc) {
            parts.push(html);
        }
    }
    parts.join("\n")
}

fn marker_type(kind: MarkerKind) -> &'static str {
    match kind {
        MarkerKind::Bullet | MarkerKind::Decimal => "1",
        MarkerKind::LowerAlpha => "a",
        MarkerKind::UpperAlpha => "A",
        MarkerKind::LowerRoman => "i",
        MarkerKind::UpperRoman => "I",
    }
}

/// Wrap a fragment in a complete, standalone HTML document.
fn wrap_document(fragment: &str) -> String {
    format!(
        "<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<style>\n{BASE_CSS}</style>\n</head>\n<body>\n{fragment}\n</body>\n</html>\n"
    )
}

/// Base stylesheet for standalone documents: readable, GitHub-like typography.
const BASE_CSS: &str = "\
body {\n  font-family: -apple-system, BlinkMacSystemFont, \"Segoe UI\", Helvetica, Arial, sans-serif;\n  line-height: 1.6;\n  color: #24292f;\n  max-width: 46em;\n  margin: 0 auto;\n  padding: 2em 1em;\n}\n\
h1, h2, h3, h4, h5, h6 { line-height: 1.25; margin: 1.5em 0 0.5em; }\n\
h1 { font-size: 2em; border-bottom: 1px solid #eaecef; padding-bottom: 0.3em; }\n\
h2 { font-size: 1.5em; border-bottom: 1px solid #eaecef; padding-bottom: 0.3em; }\n\
h3 { font-size: 1.25em; }\n\
h4 { font-size: 1em; }\n\
h5 { font-size: 0.875em; }\n\
h6 { font-size: 0.85em; color: #6a737d; }\n\
p { margin: 0 0 1em; }\n\
a { color: #0969da; text-decoration: none; }\n\
a:hover { text-decoration: underline; }\n\
code { font-family: ui-monospace, SFMono-Regular, \"SF Mono\", Menlo, Consolas, monospace; background: #f6f8fa; padding: 0.2em 0.4em; border-radius: 3px; font-size: 85%; }\n\
pre { background: #f6f8fa; padding: 1em; overflow: auto; border-radius: 6px; line-height: 1.45; }\n\
pre code { background: none; padding: 0; }\n\
blockquote { margin: 0; padding: 0 1em; color: #6a737d; border-left: 0.25em solid #d0d7de; }\n\
table { border-collapse: collapse; margin: 1em 0; display: block; overflow-x: auto; }\n\
th, td { border: 1px solid #d0d7de; padding: 6px 13px; }\n\
th { font-weight: 600; background: #f6f8fa; }\n\
hr { border: none; border-top: 1px solid #d0d7de; margin: 2em 0; }\n\
img { max-width: 100%; }\n\
ul, ol { padding-left: 2em; }\n\
.footnotes { margin-top: 2em; border-top: 1px solid #eaecef; font-size: 0.9em; color: #57606a; }\n";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Cell, CellSlot, Inline, Style, Table, TableKind};

    fn para(text: &str) -> Block {
        Block::Paragraph(vec![Inline::plain(text)])
    }

    fn styled(text: &str, style: Style) -> Inline {
        Inline::Text { text: text.to_string(), style }
    }

    #[test]
    fn headings_and_paragraphs_become_elements() {
        let doc = Document {
            blocks: vec![Block::heading(1, vec![Inline::plain("Title")]), para("Body")],
            ..Default::default()
        };
        assert_eq!(document_to_html(&doc, false), "<h1>Title</h1>\n<p>Body</p>");
    }

    #[test]
    fn heading_level_clamps_to_six() {
        let doc =
            Document { blocks: vec![Block::heading(9, vec![Inline::plain("Deep")])], ..Default::default() };
        assert_eq!(document_to_html(&doc, false), "<h6>Deep</h6>");
    }

    #[test]
    fn text_is_html_escaped() {
        let doc = Document { blocks: vec![para("<b>& \"quoted\"</b>")], ..Default::default() };
        let out = document_to_html(&doc, false);
        assert!(out.contains("&lt;b&gt;&amp;"), "escaped: {out}");
    }

    #[test]
    fn bold_and_italic_render_as_tags() {
        let bold = Style { bold: true, ..Style::PLAIN };
        let italic = Style { italic: true, ..Style::PLAIN };
        let doc = Document {
            blocks: vec![Block::Paragraph(vec![styled("b", bold), styled("i", italic)])],
            ..Default::default()
        };
        assert_eq!(document_to_html(&doc, false), "<p><strong>b</strong><em>i</em></p>");
    }

    #[test]
    fn unordered_list_renders_ul() {
        let list = List {
            marker: MarkerKind::Bullet,
            start: 1,
            items: vec![ListItem { blocks: vec![para("one")], ..Default::default() }],
        };
        let doc = Document { blocks: vec![Block::List(list)], ..Default::default() };
        assert_eq!(document_to_html(&doc, false), "<ul>\n<li>one</li>\n</ul>");
    }

    #[test]
    fn ordered_list_renders_ol_with_type() {
        let list = List {
            marker: MarkerKind::LowerRoman,
            start: 2,
            items: vec![
                ListItem { blocks: vec![para("a")], ..Default::default() },
                ListItem { blocks: vec![para("b")], ..Default::default() },
            ],
        };
        let doc = Document { blocks: vec![Block::List(list)], ..Default::default() };
        let out = document_to_html(&doc, false);
        assert!(out.contains("<ol type=\"i\" start=\"2\">"), "ol: {out}");
    }

    #[test]
    fn table_renders_colspan_rowspan() {
        let grid = vec![vec![
            CellSlot::Origin(Cell::from_inlines(vec![Inline::plain("A")])),
            CellSlot::Origin(Cell::spanning(vec![para("wide")], 2, 1)),
        ]];
        let table = Table { grid, header_rows: 1, kind: TableKind::Data };
        let doc = Document { blocks: vec![Block::Table(table)], ..Default::default() };
        let out = document_to_html(&doc, false);
        assert!(out.contains("colspan=\"2\""), "colspan: {out}");
        assert!(out.contains("<th>A</th>"), "header cell: {out}");
    }

    #[test]
    fn code_block_renders_pre_code() {
        let doc = Document {
            blocks: vec![Block::CodeBlock {
                lang: Some("rust".into()),
                text: "let x = 1;".into(),
            }],
            ..Default::default()
        };
        let out = document_to_html(&doc, false);
        assert_eq!(out, "<pre><code class=\"language-rust\">let x = 1;</code></pre>");
    }

    #[test]
    fn full_document_wraps_with_doctype() {
        let doc = Document { blocks: vec![para("hi")], ..Default::default() };
        let out = document_to_html(&doc, true);
        assert!(out.starts_with("<!DOCTYPE html>"), "full: {out}");
        assert!(out.contains("<body>"), "body: {out}");
        assert!(out.contains("<p>hi</p>"), "content: {out}");
    }
}

//! Inline rendering: model inlines become HTML phrasing content.

use crate::model::{ImageSource, Inline, LinkTarget, Style};
use crate::render::html::Ctx;
use crate::render::html::escape::{escape_attr, escape_text};
use std::fmt::Write as _;

pub(crate) fn render_inlines(inlines: &[Inline], rc: &Ctx) -> String {
    let mut out = String::new();
    for inline in inlines {
        match inline {
            Inline::Text { text, style } => render_text(text, *style, &mut out),
            Inline::Link { content, target } => render_link(content, target, rc, &mut out),
            Inline::Image { alt, source } => render_image(alt, source, &mut out),
            Inline::Anchor(id) => {
                if let Some(html_id) = rc.anchors.html_id(id) {
                    let _ = write!(out, "<a id=\"{}\"></a>", escape_attr(html_id));
                }
            }
            Inline::NoteRef(id) => {
                if let Some(num) = rc.nums.get(id.as_str()) {
                    let _ = write!(
                        out,
                        "<sup class=\"footnote-ref\"><a href=\"#fn-{num}\" id=\"fnref-{num}\">{num}</a></sup>"
                    );
                }
            }
            Inline::LineBreak => out.push_str("<br>"),
        }
    }
    out
}

/// Emit a styled run. Code runs render as `<code>` (other toggles ignored,
/// matching the Markdown renderer); otherwise strike/bold/italic nest.
fn render_text(text: &str, style: Style, out: &mut String) {
    let escaped = escape_text(text);
    if style == Style::PLAIN {
        out.push_str(&escaped);
        return;
    }
    if style.code {
        let _ = write!(out, "<code>{escaped}</code>");
        return;
    }
    if style.strike {
        out.push_str("<del>");
    }
    if style.bold {
        out.push_str("<strong>");
    }
    if style.italic {
        out.push_str("<em>");
    }
    out.push_str(&escaped);
    if style.italic {
        out.push_str("</em>");
    }
    if style.bold {
        out.push_str("</strong>");
    }
    if style.strike {
        out.push_str("</del>");
    }
}

fn render_link(content: &[Inline], target: &LinkTarget, rc: &Ctx, out: &mut String) {
    let url = match target {
        LinkTarget::External(url) | LinkTarget::Relative(url) => url.clone(),
        LinkTarget::Anchor(id) => match rc.anchors.fragment(id) {
            Some(fragment) => format!("#{fragment}"),
            None => {
                // Target exists nowhere in the document: degrade to plain text.
                log::debug!("unresolved internal link target: {id}");
                out.push_str(&render_inlines(content, rc));
                return;
            }
        },
    };
    let mut label = render_inlines(content, rc);
    if label.is_empty() {
        if matches!(target, LinkTarget::Anchor(_)) {
            return;
        }
        label = escape_text(&url);
    }
    let _ = write!(out, "<a href=\"{}\">{label}</a>", escape_attr(&url));
}

fn render_image(alt: &str, source: &ImageSource, out: &mut String) {
    match source {
        ImageSource::External(url) => {
            let _ = write!(
                out,
                "<img src=\"{}\" alt=\"{}\">",
                escape_attr(url),
                escape_attr(alt.trim())
            );
        }
        // Embedded assets render as their alt text only (same as Markdown);
        // the bytes stay available in `Document::assets`.
        ImageSource::Asset(_) | ImageSource::Unavailable => {
            if !alt.trim().is_empty() {
                out.push_str(&escape_text(alt.trim()));
            }
        }
    }
}

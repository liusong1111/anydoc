//! Table rendering over the canonical grid. Unlike Markdown (which has no
//! span syntax and renders covered positions blank), HTML expresses cell
//! spans directly as `colspan`/`rowspan`, so covered positions emit nothing.

use crate::model::{Block, Cell, CellSlot, Table};
use crate::render::html::Ctx;
use crate::render::html::inline::render_inlines;
use crate::render::html::render_block;
use std::fmt::Write as _;

pub(crate) fn render_table(table: &Table, rc: &Ctx) -> String {
    if table.grid.is_empty() {
        return String::new();
    }
    let header_rows = table.header_rows.min(table.grid.len());

    let mut out = String::new();
    out.push_str("<table>\n");
    let mut thead_open = false;
    let mut tbody_open = false;

    for (r, row) in table.grid.iter().enumerate() {
        let in_head = r < header_rows;
        if in_head && !thead_open {
            out.push_str("<thead>\n");
            thead_open = true;
        } else if !in_head && thead_open {
            out.push_str("</thead>\n");
            thead_open = false;
        }
        if !in_head && !tbody_open {
            out.push_str("<tbody>\n");
            tbody_open = true;
        }

        out.push_str("<tr>\n");
        let tag = if in_head { "th" } else { "td" };
        for slot in row {
            let CellSlot::Origin(cell) = slot else {
                // Covered position: the spanning origin carries the cell.
                continue;
            };
            let mut attrs = String::new();
            if cell.col_span > 1 {
                let _ = write!(attrs, " colspan=\"{}\"", cell.col_span);
            }
            if cell.row_span > 1 {
                let _ = write!(attrs, " rowspan=\"{}\"", cell.row_span);
            }
            let _ = writeln!(out, "<{tag}{attrs}>{}</{tag}>", render_cell(cell, rc));
        }
        out.push_str("</tr>\n");
    }

    if thead_open {
        out.push_str("</thead>\n");
    }
    if tbody_open {
        out.push_str("</tbody>\n");
    }
    out.push_str("</table>");
    out
}

/// Render a cell's blocks, joining them with `<br>` (spreadsheet/CSV cells
/// carry multiple paragraphs and line breaks this way). Paragraphs and
/// headings stay inline so cells don't carry redundant `<p>`/`<h1>` wrappers.
fn render_cell(cell: &Cell, rc: &Ctx) -> String {
    let mut parts: Vec<String> = Vec::new();
    for block in &cell.blocks {
        let html = match block {
            // Trim cell padding (spreadsheets/CSV carry it by the thousand);
            // HTML collapses edge whitespace anyway.
            Block::Paragraph(inlines) => render_inlines(inlines, rc).trim().to_string(),
            Block::Heading { content, .. } => {
                let text = render_inlines(content, rc);
                if text.trim().is_empty() { String::new() } else { format!("<strong>{text}</strong>") }
            }
            _ => render_block(block, rc).unwrap_or_default(),
        };
        if !html.is_empty() {
            parts.push(html);
        }
    }
    parts.join("<br>")
}

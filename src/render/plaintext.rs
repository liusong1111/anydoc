//! Plain text renderer: converts documents to plain text without markup.

use crate::model::{Block, Document, List, ListItem, Table, CellSlot, inlines_to_plain_text};

/// Convert a document to plain text without markup.
pub fn document_to_plaintext(doc: &Document) -> String {
    let mut out = String::new();

    for block in &doc.blocks {
        render_block(block, &mut out, 0);
        out.push_str("\n\n");
    }

    // Render footnotes/endnotes if present
    if !doc.notes.is_empty() {
        out.push_str("---\n\n");
        for (idx, note) in doc.notes.iter().enumerate() {
            out.push_str(&format!("[{}] ", idx + 1));
            for block in &note.blocks {
                render_block(block, &mut out, 0);
            }
            out.push_str("\n\n");
        }
    }

    out.trim_end().to_string() + "\n"
}

fn render_block(block: &Block, out: &mut String, indent: usize) {
    match block {
        Block::Heading { content, .. } => {
            out.push_str(&inlines_to_plain_text(content));
        }
        Block::Paragraph(inlines) => {
            let text = inlines_to_plain_text(inlines);
            if !text.trim().is_empty() {
                if indent > 0 {
                    out.push_str(&"  ".repeat(indent));
                }
                out.push_str(&text);
            }
        }
        Block::List(list) => render_list(list, out, indent),
        Block::Table(table) => render_table(table, out),
        Block::CodeBlock { text, .. } => {
            out.push_str(text);
        }
        Block::Rule => {
            out.push_str("---");
        }
        Block::BlockQuote(blocks) => {
            for (i, block) in blocks.iter().enumerate() {
                if i > 0 {
                    out.push_str("\n\n");
                }
                render_block(block, out, indent + 1);
            }
        }
    }
}

fn render_list(list: &List, out: &mut String, indent: usize) {
    let indent_str = "  ".repeat(indent);

    for (ordinal, item) in (list.start..).zip(list.items.iter()) {
        // Render marker
        out.push_str(&indent_str);
        if let Some(ref label) = item.marker_label {
            out.push_str(label);
            out.push(' ');
        } else if list.ordered() {
            out.push_str(&list.marker.label(ordinal));
            out.push(' ');
        } else {
            out.push_str("- ");
        }

        // Checkbox for task lists
        if let Some(checked) = item.checked {
            out.push_str(if checked { "[x] " } else { "[ ] " });
        }

        // Render item content
        render_list_item(item, out, indent);
        out.push('\n');
    }
}

fn render_list_item(item: &ListItem, out: &mut String, indent: usize) {
    let mut first = true;
    for block in &item.blocks {
        if !first {
            out.push('\n');
            // Indent continuation blocks
            out.push_str(&"  ".repeat(indent + 1));
        }

        match block {
            Block::Paragraph(inlines) => {
                out.push_str(&inlines_to_plain_text(inlines));
            }
            Block::List(nested_list) => {
                if !first {
                    out.push('\n');
                }
                render_list(nested_list, out, indent + 1);
            }
            _ => {
                let mut temp = String::new();
                render_block(block, &mut temp, indent + 1);
                out.push_str(&temp);
            }
        }
        first = false;
    }
}

fn render_table(table: &Table, out: &mut String) {
    // Collect cell contents and measure column widths
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut col_widths: Vec<usize> = Vec::new();

    for grid_row in &table.grid {
        let mut row_cells: Vec<String> = Vec::new();

        for (col_idx, slot) in grid_row.iter().enumerate() {
            let text = match slot {
                CellSlot::Origin(cell) => {
                    let mut cell_text = String::new();
                    for (i, block) in cell.blocks.iter().enumerate() {
                        if i > 0 {
                            cell_text.push(' ');
                        }
                        match block {
                            Block::Paragraph(inlines) => {
                                cell_text.push_str(&inlines_to_plain_text(inlines));
                            }
                            _ => {
                                let mut temp = String::new();
                                render_block(block, &mut temp, 0);
                                cell_text.push_str(temp.trim());
                            }
                        }
                    }
                    cell_text.trim().to_string()
                }
                CellSlot::Covered { .. } => String::new(),
            };

            row_cells.push(text.clone());

            // Update column width
            if col_idx >= col_widths.len() {
                col_widths.push(0);
            }
            col_widths[col_idx] = col_widths[col_idx].max(text.len());
        }

        rows.push(row_cells);
    }

    // Render rows with aligned columns
    for (row_idx, row) in rows.iter().enumerate() {
        for (col_idx, cell_text) in row.iter().enumerate() {
            if col_idx > 0 {
                out.push_str("  ");
            }
            out.push_str(cell_text);

            // Pad to column width (except last column)
            if col_idx + 1 < row.len() {
                let padding = col_widths[col_idx].saturating_sub(cell_text.len());
                out.push_str(&" ".repeat(padding));
            }
        }
        out.push('\n');

        // Add separator after header rows
        if table.header_rows > 0 && row_idx + 1 == table.header_rows {
            for (col_idx, &width) in col_widths.iter().enumerate() {
                if col_idx > 0 {
                    out.push_str("  ");
                }
                out.push_str(&"-".repeat(width.max(3)));
            }
            out.push('\n');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Cell, Inline, MarkerKind, TableKind};

    fn para(text: &str) -> Block {
        Block::Paragraph(vec![Inline::plain(text)])
    }

    #[test]
    fn headings_and_paragraphs_render_as_text() {
        let doc = Document {
            blocks: vec![
                Block::heading(1, vec![Inline::plain("Title")]),
                para("Hello world"),
            ],
            ..Default::default()
        };
        assert_eq!(document_to_plaintext(&doc), "Title\n\nHello world\n");
    }

    #[test]
    fn bullet_list_renders_plain_markers() {
        let item = ListItem {
            blocks: vec![para("one")],
            ..Default::default()
        };
        let list = List {
            marker: MarkerKind::Bullet,
            start: 1,
            items: vec![item],
        };
        let doc = Document { blocks: vec![Block::List(list)], ..Default::default() };
        assert_eq!(document_to_plaintext(&doc), "- one\n");
    }

    #[test]
    fn ordered_list_renders_numbers() {
        let items = vec![
            ListItem { blocks: vec![para("first")], ..Default::default() },
            ListItem { blocks: vec![para("second")], ..Default::default() },
        ];
        let list = List { marker: MarkerKind::Decimal, start: 1, items };
        let doc = Document { blocks: vec![Block::List(list)], ..Default::default() };
        assert_eq!(document_to_plaintext(&doc), "1. first\n2. second\n");
    }

    #[test]
    fn table_renders_aligned_columns_without_pipes() {
        let grid = vec![
            vec![
                CellSlot::Origin(Cell::from_inlines(vec![Inline::plain("Name")])),
                CellSlot::Origin(Cell::from_inlines(vec![Inline::plain("Value")])),
            ],
            vec![
                CellSlot::Origin(Cell::from_inlines(vec![Inline::plain("a")])),
                CellSlot::Origin(Cell::from_inlines(vec![Inline::plain("longer")])),
            ],
        ];
        let table = Table { grid, header_rows: 1, kind: TableKind::Data };
        let doc = Document { blocks: vec![Block::Table(table)], ..Default::default() };
        let out = document_to_plaintext(&doc);
        assert!(!out.contains('|'), "no pipes in plain text: {out}");
        assert!(out.contains("Name"), "header present: {out}");
        assert!(out.contains("longer"), "row present: {out}");
    }

    #[test]
    fn code_block_renders_verbatim() {
        let doc = Document {
            blocks: vec![Block::CodeBlock {
                lang: Some("rust".into()),
                text: "let x = 1;\nlet y = 2;".into(),
            }],
            ..Default::default()
        };
        let out = document_to_plaintext(&doc);
        assert_eq!(out, "let x = 1;\nlet y = 2;\n");
    }

    #[test]
    fn blockquote_renders_indented_paragraphs() {
        let doc = Document {
            blocks: vec![Block::BlockQuote(vec![para("quoted one"), para("quoted two")])],
            ..Default::default()
        };
        let out = document_to_plaintext(&doc);
        assert_eq!(out, "  quoted one\n\n  quoted two\n");
    }
}

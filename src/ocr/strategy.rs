//! Deciding what needs OCR: page-scan heuristics for embedded images, and
//! the document walk that applies OCR to them.
//!
//! PDF pages are not handled here — pdf-inspector already classifies them
//! (`src/formats/pdf.rs`). This module covers Office documents, where a
//! scanned page shows up as a large paper-shaped image.

use crate::model::{Asset, Block, CellSlot, Document, ImageSource, Inline};

use super::backend::{OcrBackend, OcrOptions};

/// How eagerly embedded images are treated as page scans.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OcrStrategy {
    /// Only high-confidence scans: paper-shaped, high resolution, and not
    /// sitting inside a text run. The default.
    #[default]
    Conservative,
    /// Every large-enough image, regardless of shape. Use for documents with
    /// non-standard layouts (wide screenshots, collages, multi-page stitches).
    Aggressive,
    /// Never OCR embedded images.
    Disabled,
}

/// Where an image sits in the document, for scan detection.
#[derive(Debug, Clone, Copy, Default)]
pub struct BlockContext {
    /// Inside a table cell (charts, logos — never scans).
    pub in_table: bool,
    /// The surrounding text run carries real text (an inline illustration,
    /// not a standalone scan).
    pub has_adjacent_text: bool,
}

/// Heuristic: does this asset look like a scanned document page?
///
/// `Aggressive`: any image ≥1200px on the long side, ignoring shape.
/// `Conservative`: paper-shaped (0.68–0.80 ratio) and ≥1500px, excluding
/// inline illustrations. Both modes skip images in tables.
pub fn is_document_scan(asset: &Asset, context: &BlockContext, strategy: OcrStrategy) -> bool {
    if matches!(strategy, OcrStrategy::Disabled) || context.in_table {
        return false;
    }
    if !asset.media_type.starts_with("image/") {
        return false;
    }
    if context.has_adjacent_text && matches!(strategy, OcrStrategy::Conservative) {
        return false;
    }

    let Some((width, height)) = image_dimensions(&asset.bytes) else {
        return false;
    };
    let long = width.max(height);
    let short = width.min(height);

    // Aggressive mode: skip shape check, OCR any large-enough image.
    if matches!(strategy, OcrStrategy::Aggressive) {
        return long >= 1200;
    }

    // Conservative mode: only paper-shaped images.
    // A4/B5 ≈ 0.707, Legal ≈ 0.72, Letter ≈ 0.77 (portrait or landscape).
    // 3:2 photos (0.667) fall outside; large 4:3 photos (0.75) are a known
    // false positive, mitigated by the adjacent-text exclusion.
    let ratio = short as f32 / long as f32;
    let paper_like = (0.68..=0.80).contains(&ratio);
    paper_like && long >= 1500
}

/// Read image dimensions from the header only (no full decode).
fn image_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
}

/// Walk a parsed document and replace the alt text of every image that
/// [`is_document_scan`] flags with its recognized text. The image bytes
/// stay in `Document::assets` untouched.
///
/// Failures degrade with a log, consistent with the crate-wide recovery
/// policy: one bad image never fails the conversion.
pub(crate) fn apply_to_document(
    doc: &mut Document,
    backend: &dyn OcrBackend,
    strategy: OcrStrategy,
    options: &OcrOptions,
) {
    if matches!(strategy, OcrStrategy::Disabled) {
        return;
    }
    walk_blocks(&mut doc.blocks, &doc.assets, backend, strategy, options, false);
    for note in &mut doc.notes {
        walk_blocks(&mut note.blocks, &doc.assets, backend, strategy, options, false);
    }
}

fn walk_blocks(
    blocks: &mut [Block],
    assets: &[Asset],
    backend: &dyn OcrBackend,
    strategy: OcrStrategy,
    options: &OcrOptions,
    in_table: bool,
) {
    for block in blocks {
        match block {
            Block::Paragraph(inlines) => {
                walk_inlines(inlines, assets, backend, strategy, options, in_table);
            }
            Block::Heading { content, .. } => {
                walk_inlines(content, assets, backend, strategy, options, in_table);
            }
            Block::List(list) => {
                for item in &mut list.items {
                    walk_blocks(&mut item.blocks, assets, backend, strategy, options, in_table);
                }
            }
            Block::Table(table) => {
                for slot in table.grid.iter_mut().flatten() {
                    if let CellSlot::Origin(cell) = slot {
                        walk_blocks(&mut cell.blocks, assets, backend, strategy, options, true);
                    }
                }
            }
            Block::BlockQuote(nested) => {
                walk_blocks(nested, assets, backend, strategy, options, in_table);
            }
            Block::CodeBlock { .. } | Block::Rule => {}
        }
    }
}

fn walk_inlines(
    inlines: &mut [Inline],
    assets: &[Asset],
    backend: &dyn OcrBackend,
    strategy: OcrStrategy,
    options: &OcrOptions,
    in_table: bool,
) {
    let has_adjacent_text = inlines
        .iter()
        .any(|inline| matches!(inline, Inline::Text { text, .. } if !text.trim().is_empty()));
    let context = BlockContext { in_table, has_adjacent_text };

    for inline in inlines {
        match inline {
            Inline::Image { alt, source } => {
                let ImageSource::Asset(id) = source else { continue };
                let Some(asset) = assets.get(id.0) else { continue };
                if !is_document_scan(asset, &context, strategy) {
                    continue;
                }
                match backend.recognize(&asset.bytes, options) {
                    Ok(result) if !result.text.trim().is_empty() => {
                        log::info!(
                            "OCR'd embedded image (asset {}, confidence {:.2})",
                            id.0,
                            result.confidence
                        );
                        *alt = result.text;
                    }
                    Ok(_) => {
                        log::debug!("embedded image (asset {}) OCR'd to no text", id.0);
                    }
                    Err(e) => {
                        log::warn!("OCR failed for embedded image (asset {}): {e}", id.0);
                    }
                }
            }
            Inline::Link { content, .. } => {
                walk_inlines(content, assets, backend, strategy, options, in_table);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_bytes(width: u32, height: u32) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(width, height)
            .write_to(&mut buf, image::ImageFormat::Png)
            .unwrap();
        buf.into_inner()
    }

    fn asset(width: u32, height: u32) -> Asset {
        Asset {
            id: crate::model::AssetId(0),
            media_type: "image/png".to_string(),
            origin_part: String::new(),
            bytes: png_bytes(width, height),
        }
    }

    #[test]
    fn a4_scan_is_detected() {
        // 150 DPI A4 scan: 1240×1754.
        let scan = asset(1240, 1754);
        let ctx = BlockContext::default();
        assert!(is_document_scan(&scan, &ctx, OcrStrategy::Aggressive));
        assert!(is_document_scan(&scan, &ctx, OcrStrategy::Conservative));
    }

    #[test]
    fn landscape_scan_is_detected() {
        let scan = asset(1754, 1240);
        let ctx = BlockContext::default();
        assert!(is_document_scan(&scan, &ctx, OcrStrategy::Aggressive));
    }

    #[test]
    fn photos_and_logos_are_not_scans() {
        let ctx = BlockContext::default();
        // 3:2 photo — large enough for Aggressive, but not paper-shaped for Conservative.
        assert!(is_document_scan(&asset(1500, 1000), &ctx, OcrStrategy::Aggressive));
        assert!(!is_document_scan(&asset(1500, 1000), &ctx, OcrStrategy::Conservative));
        // Square logo — too small for both.
        assert!(!is_document_scan(&asset(800, 800), &ctx, OcrStrategy::Aggressive));
        // Paper-shaped but too small for both.
        assert!(!is_document_scan(&asset(710, 1000), &ctx, OcrStrategy::Aggressive));
        assert!(!is_document_scan(&asset(710, 1000), &ctx, OcrStrategy::Conservative));
    }

    #[test]
    fn context_exclusions() {
        let scan = asset(1240, 1754);
        let in_table = BlockContext { in_table: true, has_adjacent_text: false };
        assert!(!is_document_scan(&scan, &in_table, OcrStrategy::Aggressive));

        // Aggressive ignores adjacent text, Conservative respects it.
        let inline = BlockContext { in_table: false, has_adjacent_text: true };
        assert!(!is_document_scan(&scan, &inline, OcrStrategy::Conservative));
        assert!(is_document_scan(&scan, &inline, OcrStrategy::Aggressive));

        assert!(!is_document_scan(&scan, &BlockContext::default(), OcrStrategy::Disabled));
    }

    #[test]
    fn non_images_are_not_scans() {
        let mut not_an_image = asset(1240, 1754);
        not_an_image.media_type = "application/octet-stream".to_string();
        assert!(!is_document_scan(
            &not_an_image,
            &BlockContext::default(),
            OcrStrategy::Aggressive
        ));
    }

    #[test]
    fn aggressive_ocrs_non_standard_shapes() {
        let ctx = BlockContext::default();
        // Long stitched image (ratio 0.35) — Aggressive OCRs it, Conservative rejects it.
        let long_stitch = asset(1504, 4295);
        assert!(is_document_scan(&long_stitch, &ctx, OcrStrategy::Aggressive));
        assert!(!is_document_scan(&long_stitch, &ctx, OcrStrategy::Conservative));

        // Near-square screenshot (ratio 0.92) — same behavior.
        let screenshot = asset(1498, 1633);
        assert!(is_document_scan(&screenshot, &ctx, OcrStrategy::Aggressive));
        assert!(!is_document_scan(&screenshot, &ctx, OcrStrategy::Conservative));
    }
}

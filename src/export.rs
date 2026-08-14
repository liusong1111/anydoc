//! Image export planning: which embedded images become files on disk, and the
//! relative references the renderers emit for them.
//!
//! Only large illustrations are exported; decorative elements (logos, icons,
//! dividers) and images that OCR turned into text are left out, so the output
//! stays clean.

use crate::model::{Asset, AssetId, Block, CellSlot, Document, ImageSource, Inline};
use crate::shared::assets::extension_for_media_type;
use std::collections::{HashMap, HashSet};

/// One image written to disk.
#[derive(Debug, Clone)]
pub struct ExportedImage {
    /// Filename within the images directory, e.g. `image-1.png`.
    pub filename: String,
    /// MIME type of the bytes.
    pub media_type: String,
    /// The image bytes, exactly as embedded in the source.
    pub bytes: Vec<u8>,
}

/// Where an image sits in the document structure; feeds the illustration
/// heuristic.
#[derive(Debug, Clone, Copy, Default)]
pub struct ImageContext {
    /// Adjacent to text in the same paragraph (likely an inline icon).
    pub has_adjacent_text: bool,
}

/// Decide whether an embedded image should be kept as a file: a large
/// illustration, not a decorative element.
pub fn is_exportable_illustration(asset: &Asset, context: &ImageContext) -> bool {
    if !asset.media_type.starts_with("image/") {
        return false;
    }
    let Some((width, height)) = image_dimensions(&asset.bytes) else {
        return false;
    };
    let long = width.max(height);
    let short = width.min(height);
    // Too small to be a meaningful illustration.
    if long < 400 || short < 200 {
        return false;
    }
    // Hairline divider/banner: exclude extreme aspect ratios.
    if (short as f32) / (long as f32) < 0.1 {
        return false;
    }
    // Small image sitting inline with text: an icon or decoration.
    if context.has_adjacent_text && long < 600 {
        return false;
    }
    true
}

fn image_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
}

/// Maps each exportable asset to its relative reference and collects the files
/// to write. `None` when export is not requested (`prefix` is `None`).
#[derive(Debug, Default)]
pub struct ImageExportPlan {
    /// AssetId -> relative reference string (`{prefix}/{filename}`).
    refs: HashMap<AssetId, String>,
    /// Files to write, in document order.
    pub images: Vec<ExportedImage>,
    /// Deterministic filename counter.
    counter: usize,
}

impl ImageExportPlan {
    /// AssetId -> relative reference map. Renderers look images up here.
    pub fn refs(&self) -> &HashMap<AssetId, String> {
        &self.refs
    }
}

/// Build the export plan. `ocr_consumed` holds the assets OCR turned into text
/// (they render as text, not files). `prefix` is the images directory string;
/// `None` disables export entirely.
pub fn build_export_plan(
    doc: &Document,
    ocr_consumed: &HashSet<AssetId>,
    prefix: Option<&str>,
) -> Option<ImageExportPlan> {
    let prefix = prefix?;
    let mut plan = ImageExportPlan::default();
    walk_blocks(&doc.blocks, &doc.assets, ocr_consumed, prefix, &mut plan);
    for note in &doc.notes {
        walk_blocks(&note.blocks, &doc.assets, ocr_consumed, prefix, &mut plan);
    }
    Some(plan)
}

fn walk_blocks(
    blocks: &[Block],
    assets: &[Asset],
    ocr_consumed: &HashSet<AssetId>,
    prefix: &str,
    plan: &mut ImageExportPlan,
) {
    for block in blocks {
        match block {
            Block::Paragraph(inlines) => {
                walk_inlines(inlines, assets, ocr_consumed, prefix, plan);
            }
            Block::Heading { content, .. } => {
                walk_inlines(content, assets, ocr_consumed, prefix, plan);
            }
            Block::List(list) => {
                for item in &list.items {
                    walk_blocks(&item.blocks, assets, ocr_consumed, prefix, plan);
                }
            }
            Block::Table(table) => {
                for slot in table.grid.iter().flatten() {
                    if let CellSlot::Origin(cell) = slot {
                        walk_blocks(&cell.blocks, assets, ocr_consumed, prefix, plan);
                    }
                }
            }
            Block::BlockQuote(nested) => {
                walk_blocks(nested, assets, ocr_consumed, prefix, plan);
            }
            Block::CodeBlock { .. } | Block::Rule => {}
        }
    }
}

fn walk_inlines(
    inlines: &[Inline],
    assets: &[Asset],
    ocr_consumed: &HashSet<AssetId>,
    prefix: &str,
    plan: &mut ImageExportPlan,
) {
    let has_adjacent_text = inlines
        .iter()
        .any(|inline| matches!(inline, Inline::Text { text, .. } if !text.trim().is_empty()));

    let context = ImageContext { has_adjacent_text };

    for inline in inlines {
        match inline {
            Inline::Image { source, .. } => {
                let ImageSource::Asset(id) = source else { continue; };
                if ocr_consumed.contains(id) || plan.refs.contains_key(id) {
                    continue;
                }
                let Some(asset) = assets.get(id.0) else { continue; };
                if !is_exportable_illustration(asset, &context) {
                    continue;
                }
                let Some(ext) = extension_for_media_type(&asset.media_type) else { continue; };
                plan.counter += 1;
                let filename = format!("image-{}.{ext}", plan.counter);
                let reference =
                    if prefix.is_empty() { filename.clone() } else { format!("{prefix}/{filename}") };
                plan.refs.insert(*id, reference);
                plan.images.push(ExportedImage {
                    filename,
                    media_type: asset.media_type.clone(),
                    bytes: asset.bytes.clone(),
                });
            }
            Inline::Link { content, .. } => {
                walk_inlines(content, assets, ocr_consumed, prefix, plan);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Block, Inline};

    fn png_bytes(width: u32, height: u32) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(width, height)
            .write_to(&mut buf, image::ImageFormat::Png)
            .unwrap();
        buf.into_inner()
    }

    fn asset(width: u32, height: u32) -> Asset {
        Asset {
            id: AssetId(0),
            media_type: "image/png".to_string(),
            origin_part: String::new(),
            bytes: png_bytes(width, height),
        }
    }

    #[test]
    fn too_small_is_not_exported() {
        assert!(!is_exportable_illustration(&asset(100, 100), &ImageContext::default()));
    }

    #[test]
    fn hairline_divider_is_not_exported() {
        assert!(!is_exportable_illustration(&asset(2000, 50), &ImageContext::default()));
    }

    #[test]
    fn inline_small_image_is_not_exported() {
        let ctx = ImageContext { has_adjacent_text: true };
        assert!(!is_exportable_illustration(&asset(500, 500), &ctx));
    }

    #[test]
    fn large_illustration_is_exported() {
        assert!(is_exportable_illustration(&asset(800, 600), &ImageContext::default()));
    }

    #[test]
    fn non_image_is_not_exported() {
        let mut a = asset(800, 600);
        a.media_type = "application/octet-stream".into();
        assert!(!is_exportable_illustration(&a, &ImageContext::default()));
    }

    #[test]
    fn plan_assigns_deterministic_filenames() {
        let assets = vec![
            Asset {
                id: AssetId(0),
                media_type: "image/png".to_string(),
                origin_part: "media/a.png".into(),
                bytes: png_bytes(800, 600),
            },
            Asset {
                id: AssetId(1),
                media_type: "image/jpeg".to_string(),
                origin_part: "media/b.jpg".into(),
                bytes: png_bytes(100, 100),
            },
        ];
        let doc = Document {
            blocks: vec![Block::Paragraph(vec![
                Inline::Image {
                    alt: String::new(),
                    source: ImageSource::Asset(AssetId(0)),
                },
                Inline::Image {
                    alt: String::new(),
                    source: ImageSource::Asset(AssetId(1)),
                },
            ])],
            notes: vec![],
            assets,
        };

        let plan = build_export_plan(&doc, &HashSet::new(), Some("img")).unwrap();
        // Only the large image is exported; the small one is decorative.
        assert_eq!(plan.images.len(), 1);
        assert_eq!(plan.images[0].filename, "image-1.png");
        assert_eq!(plan.refs.get(&AssetId(0)).map(String::as_str), Some("img/image-1.png"));
        assert!(!plan.refs.contains_key(&AssetId(1)));
    }

    #[test]
    fn ocr_consumed_images_are_not_exported() {
        let assets = vec![Asset {
            id: AssetId(0),
            media_type: "image/png".to_string(),
            origin_part: "media/a.png".into(),
            bytes: png_bytes(1240, 1754),
        }];
        let doc = Document {
            blocks: vec![Block::Paragraph(vec![Inline::Image {
                alt: String::new(),
                source: ImageSource::Asset(AssetId(0)),
            }])],
            notes: vec![],
            assets,
        };

        let consumed = HashSet::from([AssetId(0)]);
        let plan = build_export_plan(&doc, &consumed, Some("img")).unwrap();
        assert!(plan.images.is_empty());
    }
}

//! PDF via [pdf-inspector]: classification plus direct Markdown extraction.
//!
//! Unlike the other frontends, pdf-inspector emits Markdown itself, so PDFs
//! bypass the document model and the shared GFM writer. Scanned and
//! image-only PDFs need OCR: without a backend they error as unsupported
//! (or degrade with a log when only some pages are affected, consistent
//! with the crate-wide recovery policy); [`to_markdown_with_ocr`] renders
//! those pages with hayro and recognizes them through an
//! [`OcrBackend`](crate::ocr::OcrBackend).
//!
//! [pdf-inspector]: https://github.com/firecrawl/pdf-inspector

use crate::error::ConvertError;
use crate::ocr::{OcrBackend, OcrOptions};
use pdf_inspector::PdfError;

/// Render scale for OCR pages: 3× the PDF point size (72 dpi), i.e. 216 dpi
/// — enough for PP-OCRv5 without exploding memory on large pages.
const OCR_RENDER_SCALE: f32 = 3.0;

/// Convert a PDF to Markdown, OCR'ing scanned pages through `backend`.
///
/// Pages are assembled in document order: text pages use pdf-inspector's
/// per-page extraction, pages it flags as needing OCR are rendered to a
/// bitmap and recognized. Without a backend this behaves exactly like
/// [`to_markdown`](crate::to_markdown_bytes)'s PDF path.
pub fn to_markdown_with_ocr(
    bytes: &[u8],
    ocr: Option<&dyn OcrBackend>,
    options: &OcrOptions,
) -> Result<String, ConvertError> {
    let result = pdf_inspector::process_pdf_mem(bytes).map_err(map_error)?;
    if result.has_encoding_issues {
        log::warn!("broken font encodings detected; extracted text may be garbled");
    }

    // Fast paths: nothing to OCR, or no backend — identical to to_markdown.
    if result.pages_needing_ocr.is_empty() || ocr.is_none() {
        if ocr.is_none() && !result.pages_needing_ocr.is_empty() {
            log::warn!(
                "{} of {} pages need OCR and were not extracted",
                result.pages_needing_ocr.len(),
                result.page_count
            );
        }
        return match result.markdown {
            Some(mut markdown) if !markdown.trim().is_empty() => {
                markdown = strip_underline_tags(&markdown);
                if !markdown.ends_with('\n') {
                    markdown.push('\n');
                }
                Ok(markdown)
            }
            _ => Err(ConvertError::Unsupported(format!(
                "PDF has no extractable text ({:?}, {} pages): OCR is required",
                result.pdf_type, result.page_count
            ))),
        };
    }

    let backend = ocr.expect("checked above");
    log::info!("OCR processing {} of {} pages", result.pages_needing_ocr.len(), result.page_count);

    // Per-page extraction keeps text and OCR'd pages in document order.
    let pages = pdf_inspector::extract_pages_markdown_mem(bytes, None).map_err(map_error)?;
    let renderer = PdfRenderer::new(bytes)?;

    let mut out = String::new();
    for page in &pages.pages {
        let markdown = if page.needs_ocr {
            match renderer.ocr_page(page.page, backend, options) {
                Ok(text) if !text.trim().is_empty() => text,
                Ok(_) => {
                    log::warn!("OCR found no text on page {}", page.page + 1);
                    page.markdown.clone()
                }
                Err(e) => {
                    log::warn!("OCR failed on page {}: {e}", page.page + 1);
                    page.markdown.clone()
                }
            }
        } else {
            page.markdown.clone()
        };
        if !markdown.trim().is_empty() {
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str(markdown.trim_end());
        }
    }

    if out.trim().is_empty() {
        return Err(ConvertError::Unsupported(format!(
            "PDF has no extractable text ({:?}, {} pages), even after OCR",
            result.pdf_type, result.page_count
        )));
    }
    let mut out = strip_underline_tags(&out);
    out.push('\n');
    Ok(out)
}

/// pdf-inspector wraps geometrically-underlined text in `<u>` tags, which are
/// not GFM and read as noise downstream. Strip the tags, keep the text.
fn strip_underline_tags(markdown: &str) -> String {
    markdown.replace("<u>", "").replace("</u>", "")
}

/// hayro page renderer, created once per PDF so pages share the parse.
struct PdfRenderer {
    pdf: hayro::hayro_syntax::Pdf,
}

impl PdfRenderer {
    fn new(bytes: &[u8]) -> Result<Self, ConvertError> {
        let pdf = hayro::hayro_syntax::Pdf::new(bytes.to_vec())
            .map_err(|e| ConvertError::malformed(format!("hayro cannot read PDF: {e:?}")))?;
        Ok(Self { pdf })
    }

    /// Render one 0-indexed page and recognize it.
    fn ocr_page(
        &self,
        page_index: u32,
        backend: &dyn OcrBackend,
        options: &OcrOptions,
    ) -> Result<String, ConvertError> {
        use hayro::hayro_interpret::InterpreterSettings;

        let pages = self.pdf.pages();
        let page = pages.get(page_index as usize).ok_or_else(|| {
            ConvertError::malformed(format!("page {} missing from PDF", page_index + 1))
        })?;
        let settings = InterpreterSettings::default();
        let render_settings = hayro::RenderSettings {
            x_scale: OCR_RENDER_SCALE,
            y_scale: OCR_RENDER_SCALE,
            bg_color: hayro::vello_cpu::color::palette::css::WHITE,
            ..Default::default()
        };
        let cache = hayro::RenderCache::new();
        let pixmap = hayro::render(page, &cache, &settings, &render_settings);
        let png = pixmap
            .into_png()
            .map_err(|e| ConvertError::malformed(format!("failed to encode page render: {e}")))?;

        let result = backend
            .recognize(&png, options)
            .map_err(|e| ConvertError::Unsupported(format!("OCR failed: {e}")))?;
        if result.confidence < 0.8 {
            log::warn!("low OCR confidence on page {}: {:.2}", page_index + 1, result.confidence);
        }
        Ok(result.text)
    }
}

fn map_error(e: PdfError) -> ConvertError {
    match e {
        PdfError::Encrypted => ConvertError::Encrypted,
        PdfError::Io(e) => ConvertError::Io(e),
        PdfError::NotAPdf(detail) => ConvertError::malformed(format!("not a PDF: {detail}")),
        PdfError::InvalidStructure => ConvertError::malformed("invalid PDF structure"),
        PdfError::Parse(detail) => ConvertError::malformed(detail),
    }
}

#[cfg(test)]
mod tests {
    use super::strip_underline_tags;

    #[test]
    fn strips_underline_tags_but_keeps_text() {
        let input = "Relative link to <u>a sibling file</u>. Jump to <u>the bookmark</u>.\n";
        assert_eq!(
            strip_underline_tags(input),
            "Relative link to a sibling file. Jump to the bookmark.\n"
        );
    }

    #[test]
    fn leaves_plain_markdown_untouched() {
        let input = "# Title\n\nSome **bold** and [a link](https://x.y).\n";
        assert_eq!(strip_underline_tags(input), input);
    }
}

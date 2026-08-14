//! End-to-end OCR tests.
//!
//! Fixtures live in `tests/fixtures-ocr/` (regenerate with
//! `scripts/make-ocr-fixtures.py`). Tests that exercise the OCR engine need
//! the PP-OCRv5 model files in `models/` (`scripts/download-models.sh`);
//! without them they skip, so a plain checkout still passes `cargo test`.

use std::path::Path;

use anydoc::ocr::{EmbeddedOcrBackend, OcrStrategy};
use anydoc::{ConvertError, Format, OutputFormat, to_markdown_bytes, to_markdown_bytes_with_ocr, to_output_with_ocr};

fn fixture(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures-ocr").join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read {path:?}: {e}"))
}

/// Load the OCR engine, or skip the calling test when the model files are
/// not installed.
fn backend() -> Option<EmbeddedOcrBackend> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("models");
    match EmbeddedOcrBackend::from_model_dir(&dir) {
        Ok(backend) => Some(backend),
        Err(e) => {
            eprintln!("skipping: OCR models unavailable ({e}); run scripts/download-models.sh");
            None
        }
    }
}

#[test]
fn text_pdf_converts_without_ocr() {
    let markdown = to_markdown_bytes(&fixture("text.pdf"), Format::Pdf).unwrap();
    assert!(markdown.contains("Hello from a plain text PDF page."));
}

#[test]
fn scanned_pdf_errors_without_ocr() {
    let result = to_markdown_bytes(&fixture("scanned.pdf"), Format::Pdf);
    match result {
        Err(ConvertError::Unsupported(detail)) => {
            assert!(detail.contains("OCR is required"), "unexpected detail: {detail}");
        }
        other => panic!("expected Unsupported, got {other:?}"),
    }
}

#[test]
fn scanned_pdf_with_ocr() {
    let Some(backend) = backend() else { return };
    let markdown = to_markdown_bytes_with_ocr(
        &fixture("scanned.pdf"),
        Format::Pdf,
        Some(&backend),
        OcrStrategy::Conservative,
    )
    .unwrap();
    assert!(markdown.contains("智能文档转换系统"), "got: {markdown}");
    assert!(markdown.contains("Any2md OCR Integration Test"), "got: {markdown}");
}

#[test]
fn mixed_pdf_keeps_pages_in_order() {
    let Some(backend) = backend() else { return };
    let markdown = to_markdown_bytes_with_ocr(
        &fixture("mixed.pdf"),
        Format::Pdf,
        Some(&backend),
        OcrStrategy::Conservative,
    )
    .unwrap();
    let text_page = markdown.find("This page is real text, no OCR needed.");
    let ocr_page = markdown.find("智能文档转换系统");
    assert!(text_page.is_some(), "text page missing: {markdown}");
    assert!(ocr_page.is_some(), "OCR page missing: {markdown}");
    assert!(text_page < ocr_page, "pages out of order: {markdown}");
}

#[test]
fn docx_scan_image_ocr_into_alt_text() {
    let bytes = fixture("scan_image.docx");

    // Without OCR the scan's text never appears.
    let plain = to_markdown_bytes(&bytes, Format::Docx).unwrap();
    assert!(!plain.contains("智能文档转换系统"));

    let Some(backend) = backend() else { return };
    let markdown =
        to_markdown_bytes_with_ocr(&bytes, Format::Docx, Some(&backend), OcrStrategy::Conservative)
            .unwrap();
    assert!(markdown.contains("智能文档转换系统"), "got: {markdown}");
}

#[test]
fn disabled_strategy_never_ocrs() {
    let Some(backend) = backend() else { return };
    let markdown = to_markdown_bytes_with_ocr(
        &fixture("scan_image.docx"),
        Format::Docx,
        Some(&backend),
        OcrStrategy::Disabled,
    )
    .unwrap();
    assert!(!markdown.contains("智能文档转换系统"), "got: {markdown}");
}

#[test]
fn docx_image_exports_as_file_reference_without_ocr() {
    // Without OCR the embedded scan image is just a large image: it is
    // exported as a file and referenced, not OCR'd.
    let out = to_output_with_ocr(
        &fixture("scan_image.docx"),
        Format::Docx,
        OutputFormat::Markdown,
        None,
        OcrStrategy::Disabled,
        Some("images"),
    )
    .unwrap();
    assert!(out.content.contains("!["), "no image reference: {}", out.content);
    assert!(out.content.contains("images/image-1."), "got: {}", out.content);
    assert_eq!(out.images.len(), 1);
    assert!(out.images[0].filename.starts_with("image-1."));
    assert!(!out.content.contains("智能文档转换系统"));
}

#[test]
fn no_prefix_means_no_image_reference() {
    let out = to_output_with_ocr(
        &fixture("scan_image.docx"),
        Format::Docx,
        OutputFormat::Markdown,
        None,
        OcrStrategy::Disabled,
        None,
    )
    .unwrap();
    assert!(!out.content.contains("!["), "got: {}", out.content);
    assert!(out.images.is_empty());
}

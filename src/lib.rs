//! anydoc converts documents to GitHub-Flavored Markdown.
//!
//! Recovery and skipped-content events are reported through the [`log`]
//! facade (debug/warn level); logging never changes conversion behavior and
//! its messages are not a stable API.

#![warn(missing_docs)]

pub mod model;
pub mod ocr;
pub mod output;
#[cfg(feature = "server")]
pub mod server;

mod error;
mod export;
mod formats;
mod package;
mod render;
mod shared;

pub use error::ConvertError;
pub use export::ExportedImage;
pub use output::OutputFormat;

use render::html::{document_to_html_with_images};
use render::markdown::document_to_markdown_with_images;
use render::plaintext::document_to_plaintext;

use std::collections::HashSet;
use std::path::Path;

/// Input format. Selects the parser; container variants that share a parser
/// (docm, xlsm, ...) map onto these via [`Format::from_bytes`] or
/// [`Format::from_extension`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    /// Binary Word 97-2003 (`.doc`).
    Doc,
    /// WordprocessingML (`.docx`, `.docm`), both Transitional and Strict.
    Docx,
    /// OpenDocument Text (`.odt`).
    Odt,
    /// Converted with [pdf-inspector], which emits Markdown directly:
    /// [`to_document`] is unsupported for PDFs. Scanned/image-only PDFs
    /// (needing OCR) error as unsupported.
    ///
    /// [pdf-inspector]: https://github.com/firecrawl/pdf-inspector
    Pdf,
    /// Binary PowerPoint 97-2003 (`.ppt`, `.pps`, `.pot`).
    Ppt,
    /// PresentationML (`.pptx`, `.pptm`, `.ppsx`, `.ppsm`).
    Pptx,
    /// Rich Text Format (`.rtf`).
    Rtf,
    /// EPUB 2 and 3 (`.epub`).
    Epub,
    /// Excel workbooks in every container calamine reads: `.xlsx`, `.xlsm`,
    /// `.xlsb`, and binary `.xls`.
    Excel,
    /// OpenDocument Spreadsheet (`.ods`).
    Ods,
    /// OpenDocument Presentation (`.odp`).
    Odp,
    /// Delimiter-separated text (`.csv`). Carries no signature, so it has to
    /// be named rather than detected.
    Csv,
}

impl Format {
    /// Detect the format from the content itself: the signature and identity
    /// each container specification designates (PDF header, RTF open group,
    /// OLE stream names, ZIP package mimetype/content types). Plain-text
    /// formats (CSV) carry no signature and return `None`; so does anything
    /// unrecognized.
    pub fn from_bytes(bytes: &[u8]) -> Option<Format> {
        formats::detect::from_bytes(bytes)
    }

    /// The format a bare extension names (no leading dot), matched
    /// case-insensitively. `None` for anything unrecognized.
    pub fn from_extension(ext: &str) -> Option<Format> {
        Some(match ext.to_ascii_lowercase().as_str() {
            "doc" => Format::Doc,
            "docx" | "docm" => Format::Docx,
            "odt" => Format::Odt,
            "pdf" => Format::Pdf,
            "pptx" | "pptm" | "ppsx" | "ppsm" => Format::Pptx,
            "ppt" | "pps" | "pot" => Format::Ppt,
            "rtf" => Format::Rtf,
            "epub" => Format::Epub,
            "xlsx" | "xlsm" | "xlsb" | "xls" => Format::Excel,
            "ods" => Format::Ods,
            "odp" => Format::Odp,
            "csv" => Format::Csv,
            _ => return None,
        })
    }

    /// The format a path's extension names. `None` when the path has no
    /// extension or names nothing recognized.
    pub fn from_path(path: &Path) -> Option<Format> {
        path.extension().and_then(|e| e.to_str()).and_then(Format::from_extension)
    }
}

/// Convert a document file to Markdown. The format is detected from the
/// file content ([`Format::from_bytes`]); the extension is the fallback for
/// signature-less formats (CSV) and unrecognizable containers.
pub fn to_markdown(path: impl AsRef<Path>) -> Result<String, ConvertError> {
    let path = path.as_ref();
    let bytes = std::fs::read(path)?;
    let Some(format) = Format::from_bytes(&bytes).or_else(|| Format::from_path(path)) else {
        return Err(ConvertError::Unsupported(format!(
            "unrecognized file content and extension: {}",
            path.display()
        )));
    };
    to_markdown_bytes(&bytes, format)
}

/// Convert an in-memory document to Markdown. Pass a [`Format`] to select the
/// parser, or `None` to detect it from the content ([`Format::from_bytes`]),
/// which signature-less formats (CSV) have to name explicitly.
pub fn to_markdown_bytes(
    bytes: &[u8],
    format: impl Into<Option<Format>>,
) -> Result<String, ConvertError> {
    to_output_with_ocr(bytes, format, OutputFormat::Markdown, None, ocr::OcrStrategy::Disabled, None)
        .map(|out| out.content)
}

/// Parse an in-memory document into the document model. Pass a [`Format`] to
/// select the parser, or `None` to detect it from the content.
///
/// Unsupported for [`Format::Pdf`]: PDF conversion produces Markdown
/// directly and has no document-model form; use [`to_markdown_bytes`].
pub fn to_document(
    bytes: &[u8],
    format: impl Into<Option<Format>>,
) -> Result<model::Document, ConvertError> {
    formats::parse(bytes, resolve_format(bytes, format.into())?)
}

/// Convert a document file to Markdown, OCR'ing scanned content when a
/// backend is given. Detection and format handling match [`to_markdown`].
///
/// Without a backend the behavior is exactly [`to_markdown`]'s. With one:
/// scanned PDF pages are rendered and recognized, and embedded images that
/// look like page scans (per `strategy`) get their recognized text as alt
/// text. See [`ocr`] for backends and options.
pub fn to_markdown_with_ocr(
    path: impl AsRef<Path>,
    ocr: Option<&dyn ocr::OcrBackend>,
    strategy: ocr::OcrStrategy,
) -> Result<String, ConvertError> {
    let path = path.as_ref();
    let bytes = std::fs::read(path)?;
    let Some(format) = Format::from_bytes(&bytes).or_else(|| Format::from_path(path)) else {
        return Err(ConvertError::Unsupported(format!(
            "unrecognized file content and extension: {}",
            path.display()
        )));
    };
    to_markdown_bytes_with_ocr(&bytes, format, ocr, strategy)
}

/// Convert an in-memory document to Markdown, OCR'ing scanned content when
/// a backend is given. The bytes/format/None-detection contract matches
/// [`to_markdown_bytes`].
pub fn to_markdown_bytes_with_ocr(
    bytes: &[u8],
    format: impl Into<Option<Format>>,
    ocr: Option<&dyn ocr::OcrBackend>,
    strategy: ocr::OcrStrategy,
) -> Result<String, ConvertError> {
    to_output_bytes_with_ocr(bytes, format, OutputFormat::Markdown, ocr, strategy)
}

/// Convert an in-memory document to the specified output format, OCR'ing
/// scanned content when a backend is given. The bytes/format/None-detection
/// contract matches [`to_markdown_bytes`].
pub fn to_output_bytes_with_ocr(
    bytes: &[u8],
    format: impl Into<Option<Format>>,
    output_format: OutputFormat,
    ocr: Option<&dyn ocr::OcrBackend>,
    strategy: ocr::OcrStrategy,
) -> Result<String, ConvertError> {
    to_output_with_ocr(bytes, format, output_format, ocr, strategy, None).map(|out| out.content)
}

/// The result of a conversion that also exports embedded images.
#[derive(Debug)]
pub struct ConversionOutput {
    /// The rendered document.
    pub content: String,
    /// Images exported as files, in document order. Empty unless `image_prefix`
    /// was provided.
    pub images: Vec<ExportedImage>,
}

/// Convert an in-memory document to the specified output format, OCR'ing
/// scanned content when a backend is given, and optionally exporting embedded
/// illustrations as image files.
///
/// `image_prefix` is the directory string prepended to each exported image's
/// filename in the rendered references (e.g. `"images"` yields
/// `![...](images/image-1.png)`); `None` disables export and leaves embedded
/// images as alt text, exactly like [`to_output_bytes_with_ocr`]. When set,
/// `images` holds the files to write, each named `image-N.ext`. Only large
/// illustrations are exported — decorative elements and images that OCR turned
/// into text are left out.
pub fn to_output_with_ocr(
    bytes: &[u8],
    format: impl Into<Option<Format>>,
    output_format: OutputFormat,
    ocr: Option<&dyn ocr::OcrBackend>,
    strategy: ocr::OcrStrategy,
    image_prefix: Option<&str>,
) -> Result<ConversionOutput, ConvertError> {
    let format = resolve_format(bytes, format.into())?;
    if format == Format::Pdf {
        // PDFs convert to Markdown directly (pdf-inspector) and never pass
        // through the document model, so non-Markdown outputs derive from the
        // Markdown: plain text strips markup, HTML goes through a Markdown
        // parser. PDFs carry no document-model assets, so no image export.
        let markdown = formats::pdf::to_markdown_with_ocr(bytes, ocr, &ocr::OcrOptions::default())?;
        let content = match output_format {
            OutputFormat::Markdown => markdown,
            OutputFormat::PlainText => markdown_to_plaintext_fallback(&markdown),
            OutputFormat::Html => markdown_to_html(&markdown),
            OutputFormat::HtmlDocument => wrap_html_document(&markdown_to_html(&markdown)),
        };
        return Ok(ConversionOutput { content, images: Vec::new() });
    }
    let mut document = to_document(bytes, format)?;
    let ocr_consumed = match ocr {
        Some(backend) => {
            ocr::apply_to_document(&mut document, backend, strategy, &ocr::OcrOptions::default())
        }
        None => HashSet::new(),
    };
    // Images only make sense in formats that can reference them.
    let plan = if matches!(
        output_format,
        OutputFormat::Markdown | OutputFormat::Html | OutputFormat::HtmlDocument
    ) {
        export::build_export_plan(&document, &ocr_consumed, image_prefix)
    } else {
        None
    };
    let content = render_document(&document, output_format, plan.as_ref());
    let images = plan.map(|p| p.images).unwrap_or_default();
    Ok(ConversionOutput { content, images })
}

/// Render a document to the specified output format.
fn render_document(
    doc: &model::Document,
    output_format: OutputFormat,
    plan: Option<&export::ImageExportPlan>,
) -> String {
    match output_format {
        OutputFormat::Markdown => document_to_markdown_with_images(doc, plan),
        OutputFormat::PlainText => document_to_plaintext(doc),
        OutputFormat::Html => document_to_html_with_images(doc, false, plan),
        OutputFormat::HtmlDocument => document_to_html_with_images(doc, true, plan),
    }
}

/// Convert Markdown to an HTML fragment with pulldown-cmark, enabling the GFM
/// extensions (tables, strikethrough, task lists, footnotes) that the
/// Markdown renderer emits.
fn markdown_to_html(markdown: &str) -> String {
    let mut options = pulldown_cmark::Options::empty();
    options.insert(pulldown_cmark::Options::ENABLE_TABLES);
    options.insert(pulldown_cmark::Options::ENABLE_STRIKETHROUGH);
    options.insert(pulldown_cmark::Options::ENABLE_TASKLISTS);
    options.insert(pulldown_cmark::Options::ENABLE_FOOTNOTES);
    let parser = pulldown_cmark::Parser::new_ext(markdown, options);
    let mut out = String::with_capacity(markdown.len());
    pulldown_cmark::html::push_html(&mut out, parser);
    out
}

/// Wrap an HTML fragment in a minimal standalone document (PDF path only:
/// non-PDF HTML uses the renderer's own wrapper and stylesheet).
fn wrap_html_document(fragment: &str) -> String {
    format!(
        "<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n</head>\n<body>\n{fragment}\n</body>\n</html>\n"
    )
}

/// Fallback for converting markdown text to plain text (used for PDF path).
/// This is a simple approach that strips common markdown syntax.
fn markdown_to_plaintext_fallback(markdown: &str) -> String {
    let mut out = String::new();
    for line in markdown.lines() {
        let trimmed = line.trim();
        // Strip heading markers
        let line = if let Some(stripped) = trimmed.strip_prefix("######") {
            stripped.trim()
        } else if let Some(stripped) = trimmed.strip_prefix("#####") {
            stripped.trim()
        } else if let Some(stripped) = trimmed.strip_prefix("####") {
            stripped.trim()
        } else if let Some(stripped) = trimmed.strip_prefix("###") {
            stripped.trim()
        } else if let Some(stripped) = trimmed.strip_prefix("##") {
            stripped.trim()
        } else if let Some(stripped) = trimmed.strip_prefix("#") {
            stripped.trim()
        } else {
            trimmed
        };

        out.push_str(line);
        out.push('\n');
    }
    out
}

fn resolve_format(bytes: &[u8], format: Option<Format>) -> Result<Format, ConvertError> {
    format.or_else(|| Format::from_bytes(bytes)).ok_or_else(|| {
        ConvertError::Unsupported("unrecognized file content: name the format explicitly".into())
    })
}

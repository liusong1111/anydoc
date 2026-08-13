//! Output format selection.

use std::str::FromStr;

/// Output format for conversion results.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum OutputFormat {
    /// GitHub-Flavored Markdown (default).
    #[default]
    Markdown,
    /// Plain text without markup.
    PlainText,
    /// HTML fragment: block-level elements only, no document wrapper.
    Html,
    /// Complete HTML document: `<!DOCTYPE html>` + `<head>` (with base CSS)
    /// + `<body>`.
    HtmlDocument,
}

impl FromStr for OutputFormat {
    type Err = ();

    /// Parse case-insensitively: `"markdown"`/`"md"`,
    /// `"plain"`/`"text"`/`"plaintext"`/`"txt"`,
    /// `"html"`/`"htm"` (fragment), and `"html-full"`/`"html-doc"` (document).
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "markdown" | "md" => Ok(Self::Markdown),
            "plain" | "text" | "plaintext" | "txt" => Ok(Self::PlainText),
            "html" | "htm" => Ok(Self::Html),
            "html-full" | "html-doc" | "htmldoc" => Ok(Self::HtmlDocument),
            _ => Err(()),
        }
    }
}

impl OutputFormat {
    /// The canonical name for this format.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Markdown => "markdown",
            Self::PlainText => "plain",
            Self::Html => "html",
            Self::HtmlDocument => "html-doc",
        }
    }

    /// True for HTML output, fragment or full document.
    pub fn is_html(&self) -> bool {
        matches!(self, Self::Html | Self::HtmlDocument)
    }
}

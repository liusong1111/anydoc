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
}

impl FromStr for OutputFormat {
    type Err = ();

    /// Parse case-insensitively: `"markdown"`/`"md"` and
    /// `"plain"`/`"text"`/`"plaintext"`/`"txt"`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "markdown" | "md" => Ok(Self::Markdown),
            "plain" | "text" | "plaintext" | "txt" => Ok(Self::PlainText),
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
        }
    }
}

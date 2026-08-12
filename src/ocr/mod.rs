//! Optional OCR support — the any2md extension over upstream anydoc.
//!
//! OCR is entirely opt-in: without an [`OcrBackend`] the conversion paths
//! behave exactly like upstream. With one, scanned PDF pages are rendered
//! and recognized, and embedded document images that look like page scans
//! (see [`strategy`]) get their recognized text as alt text.

mod backend;
mod embedded;
mod image_features;
mod sampling;
mod strategy;

pub use backend::{BoundingBox, OcrBackend, OcrError, OcrOptions, OcrResult};
pub use embedded::EmbeddedOcrBackend;
pub use image_features::{ImageFeatures, TextLikelihood};
pub use sampling::SamplingOcrOptions;
pub use strategy::{BlockContext, OcrStrategy};

pub(crate) use strategy::apply_to_document;

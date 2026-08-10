//! OCR backend abstraction.
//!
//! Defines the [`OcrBackend`] trait used by the PDF and Office frontends,
//! plus the shared option/result/error types. The embedded implementation
//! (PP-OCRv5 via MNN) lives in [`crate::ocr::embedded`].

/// OCR engine contract. Implementations must be safe to share across
/// threads; interior mutability is the implementation's concern.
pub trait OcrBackend: Send + Sync {
    /// Recognize the text in one image (PNG/JPEG/... bytes).
    fn recognize(&self, image: &[u8], options: &OcrOptions) -> Result<OcrResult, OcrError>;

    /// Recognize several images. The default loops over [`Self::recognize`];
    /// implementations may override with a parallel or batched strategy.
    fn recognize_batch(
        &self,
        images: &[&[u8]],
        options: &OcrOptions,
    ) -> Result<Vec<OcrResult>, OcrError> {
        images
            .iter()
            .map(|image| self.recognize(image, options))
            .collect()
    }

    /// Verify the backend is ready (model loaded, service reachable).
    fn health_check(&self) -> Result<(), OcrError>;
}

/// Per-request OCR options.
#[derive(Debug, Clone)]
pub struct OcrOptions {
    /// Language hint: `zh`, `en`, `ja`, `multi`. Model-dependent; the
    /// default PP-OCRv5 model already covers Chinese/English/Japanese.
    pub language: String,
    /// Whether to detect and correct page orientation.
    pub detect_orientation: bool,
    /// Images larger than this are downscaled before recognition
    /// (speed/memory trade-off). `None` disables the limit.
    pub max_size: Option<(u32, u32)>,
}

impl Default for OcrOptions {
    fn default() -> Self {
        Self {
            language: "zh".to_string(),
            detect_orientation: true,
            max_size: Some((4096, 4096)),
        }
    }
}

/// Recognition output for one image.
#[derive(Debug, Clone, Default)]
pub struct OcrResult {
    /// Recognized text, one line per detected text region, in reading order.
    pub text: String,
    /// Mean confidence across regions, 0.0–1.0 (0.0 when no text was found).
    pub confidence: f32,
    /// Per-region details, in reading order.
    pub boxes: Vec<BoundingBox>,
}

/// One detected text region.
#[derive(Debug, Clone)]
pub struct BoundingBox {
    /// Left edge, in pixels.
    pub x: f32,
    /// Top edge, in pixels.
    pub y: f32,
    /// Region width, in pixels.
    pub width: f32,
    /// Region height, in pixels.
    pub height: f32,
    /// Recognized text of this region.
    pub text: String,
    /// Region confidence, 0.0–1.0.
    pub confidence: f32,
}

/// OCR failures.
#[derive(Debug)]
pub enum OcrError {
    /// Model files missing, unreadable, or rejected by the engine.
    InitFailed(String),
    /// The input bytes are not a decodable image.
    InvalidImage(String),
    /// The engine failed during detection or recognition.
    RecognitionFailed(String),
}

impl std::fmt::Display for OcrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OcrError::InitFailed(detail) => write!(f, "OCR initialization failed: {detail}"),
            OcrError::InvalidImage(detail) => write!(f, "invalid image: {detail}"),
            OcrError::RecognitionFailed(detail) => write!(f, "OCR recognition failed: {detail}"),
        }
    }
}

impl std::error::Error for OcrError {}

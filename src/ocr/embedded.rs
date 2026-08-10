//! Embedded OCR backend: PP-OCRv5-FP16 via [ocr-rs] (rust-paddle-ocr),
//! MNN inference, in-process on CPU. The engine is `Send + Sync` and its
//! entry points take `&self`, so the backend is a plain wrapper.
//!
//! [ocr-rs]: https://github.com/zibo-chen/rust-paddle-ocr

use std::path::Path;

use ocr_rs::{OcrEngine, OcrEngineConfig};

use super::backend::{BoundingBox, OcrBackend, OcrError, OcrOptions, OcrResult};

/// Detection model file name within a model directory.
pub const DET_MODEL_FILE: &str = "PP-OCRv5_mobile_det_fp16.mnn";
/// Recognition model file name within a model directory.
pub const REC_MODEL_FILE: &str = "PP-OCRv5_mobile_rec_fp16.mnn";
/// Character set file name within a model directory.
pub const KEYS_FILE: &str = "ppocr_keys_v5.txt";

/// Text lines whose top edges are closer than this many pixels are treated
/// as belonging to the same line when sorting into reading order.
const LINE_TOLERANCE_PX: i32 = 20;

/// In-process OCR backend wrapping `ocr_rs::OcrEngine`.
pub struct EmbeddedOcrBackend {
    engine: OcrEngine,
}

impl EmbeddedOcrBackend {
    /// Load the default PP-OCRv5-FP16 model set from a directory, with the
    /// engine's default thread count.
    pub fn from_model_dir(dir: impl AsRef<Path>) -> Result<Self, OcrError> {
        Self::from_model_dir_with_threads(dir, None)
    }

    /// Load from a directory, overriding the engine's inference thread count.
    pub fn from_model_dir_with_threads(
        dir: impl AsRef<Path>,
        threads: Option<u32>,
    ) -> Result<Self, OcrError> {
        let dir = dir.as_ref();
        Self::from_files(
            dir.join(DET_MODEL_FILE),
            dir.join(REC_MODEL_FILE),
            dir.join(KEYS_FILE),
            threads,
        )
    }

    /// Load models from explicit file paths.
    pub fn from_files(
        det_model: impl AsRef<Path>,
        rec_model: impl AsRef<Path>,
        keys: impl AsRef<Path>,
        threads: Option<u32>,
    ) -> Result<Self, OcrError> {
        let config = threads.map(|n| OcrEngineConfig::new().with_threads(n as i32));
        let engine = OcrEngine::new(det_model.as_ref(), rec_model.as_ref(), keys.as_ref(), config)
            .map_err(|e| OcrError::InitFailed(e.to_string()))?;
        Ok(Self { engine })
    }
}

impl OcrBackend for EmbeddedOcrBackend {
    fn recognize(&self, image: &[u8], options: &OcrOptions) -> Result<OcrResult, OcrError> {
        let img =
            image::load_from_memory(image).map_err(|e| OcrError::InvalidImage(e.to_string()))?;
        let img = downscale_to(img, options.max_size);

        let mut results =
            self.engine.recognize(&img).map_err(|e| OcrError::RecognitionFailed(e.to_string()))?;

        // Reading order: top to bottom; boxes on the same line left to right.
        results.sort_by(|a, b| {
            let (ra, rb) = (&a.bbox.rect, &b.bbox.rect);
            if (ra.top() - rb.top()).abs() < LINE_TOLERANCE_PX {
                ra.left().cmp(&rb.left())
            } else {
                ra.top().cmp(&rb.top())
            }
        });

        let boxes: Vec<BoundingBox> = results
            .into_iter()
            .filter(|r| !r.text.trim().is_empty())
            .map(|r| BoundingBox {
                x: r.bbox.rect.left() as f32,
                y: r.bbox.rect.top() as f32,
                width: r.bbox.rect.width() as f32,
                height: r.bbox.rect.height() as f32,
                text: r.text,
                confidence: r.confidence,
            })
            .collect();

        let text = boxes.iter().map(|b| b.text.as_str()).collect::<Vec<_>>().join("\n");
        let confidence = if boxes.is_empty() {
            0.0
        } else {
            boxes.iter().map(|b| b.confidence).sum::<f32>() / boxes.len() as f32
        };

        Ok(OcrResult { text, confidence, boxes })
    }

    fn health_check(&self) -> Result<(), OcrError> {
        // Models were loaded at construction; nothing else can fail later.
        Ok(())
    }
}

/// Downscale proportionally when either dimension exceeds `max_size`.
fn downscale_to(img: image::DynamicImage, max_size: Option<(u32, u32)>) -> image::DynamicImage {
    let Some((max_w, max_h)) = max_size else { return img };
    if img.width() <= max_w && img.height() <= max_h {
        return img;
    }
    let scale = (max_w as f32 / img.width() as f32).min(max_h as f32 / img.height() as f32);
    img.resize(
        (img.width() as f32 * scale) as u32,
        (img.height() as f32 * scale) as u32,
        image::imageops::FilterType::Lanczos3,
    )
}

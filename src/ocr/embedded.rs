//! Embedded OCR backend: PP-OCRv5-FP16 via [rust-paddle-ocr] (MNN
//! inference), running in-process on CPU.
//!
//! `Det`/`Rec` hold raw MNN pointers and are neither `Send` nor `Sync`, so
//! — the same pattern the crate's own `OcrEngine` uses — the models live on
//! a dedicated worker thread and calls go over a channel. We run our own
//! worker rather than wrapping `OcrEngine` because the engine's public API
//! returns plain strings without per-region confidence or boxes.
//!
//! [rust-paddle-ocr]: https://github.com/zibo-chen/rust-paddle-ocr

use std::path::Path;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;

use rust_paddle_ocr::{Det, Rec};

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

/// In-process OCR backend; see the module docs for the threading model.
pub struct EmbeddedOcrBackend {
    // `Option` so `Drop` can disconnect the channel before joining.
    request_tx: Option<Sender<OcrRequest>>,
    worker: Option<JoinHandle<()>>,
}

enum OcrRequest {
    Recognize {
        image: image::DynamicImage,
        respond: Sender<Result<OcrResult, OcrError>>,
    },
}

impl EmbeddedOcrBackend {
    /// Load the default PP-OCRv5-FP16 model set from a directory.
    pub fn from_model_dir(dir: impl AsRef<Path>) -> Result<Self, OcrError> {
        let dir = dir.as_ref();
        Self::from_files(
            dir.join(DET_MODEL_FILE),
            dir.join(REC_MODEL_FILE),
            dir.join(KEYS_FILE),
        )
    }

    /// Load models from explicit file paths.
    pub fn from_files(
        det_model: impl AsRef<Path>,
        rec_model: impl AsRef<Path>,
        keys: impl AsRef<Path>,
    ) -> Result<Self, OcrError> {
        let det_model = det_model.as_ref().to_path_buf();
        let rec_model = rec_model.as_ref().to_path_buf();
        let keys = keys.as_ref().to_path_buf();

        let (request_tx, request_rx) = channel::<OcrRequest>();
        let (init_tx, init_rx) = channel::<Result<(), OcrError>>();

        let worker = std::thread::spawn(move || {
            // rect_border_size 12 / no box merging: the combination upstream
            // recommends for PP-OCRv5.
            let models = Det::from_file(&det_model)
                .map(|det| det.with_rect_border_size(12).with_merge_boxes(false).with_merge_threshold(1))
                .and_then(|det| Rec::from_file(&rec_model, &keys).map(|rec| (det, rec)));
            match models {
                Ok((det, rec)) => {
                    let _ = init_tx.send(Ok(()));
                    run_worker(det, rec, request_rx);
                }
                Err(e) => {
                    let _ = init_tx.send(Err(OcrError::InitFailed(e.to_string())));
                }
            }
        });

        match init_rx.recv() {
            Ok(Ok(())) => Ok(Self { request_tx: Some(request_tx), worker: Some(worker) }),
            Ok(Err(e)) => {
                let _ = worker.join();
                Err(e)
            }
            Err(_) => {
                let _ = worker.join();
                Err(OcrError::InitFailed("OCR worker thread died during init".into()))
            }
        }
    }
}

impl Drop for EmbeddedOcrBackend {
    fn drop(&mut self) {
        // Disconnecting the channel ends the worker's receive loop.
        drop(self.request_tx.take());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl OcrBackend for EmbeddedOcrBackend {
    fn recognize(&self, image: &[u8], options: &OcrOptions) -> Result<OcrResult, OcrError> {
        let img = image::load_from_memory(image)
            .map_err(|e| OcrError::InvalidImage(e.to_string()))?;
        let img = downscale_to(img, options.max_size);

        let (respond, result_rx) = channel();
        let request_tx = self
            .request_tx
            .as_ref()
            .ok_or_else(|| OcrError::RecognitionFailed("OCR worker thread has terminated".into()))?;
        request_tx
            .send(OcrRequest::Recognize { image: img, respond })
            .map_err(|_| OcrError::RecognitionFailed("OCR worker thread has terminated".into()))?;
        result_rx
            .recv()
            .map_err(|_| OcrError::RecognitionFailed("OCR worker thread has terminated".into()))?
    }

    fn health_check(&self) -> Result<(), OcrError> {
        // Models were loaded at construction; nothing else can fail later.
        Ok(())
    }
}

fn run_worker(mut det: Det, mut rec: Rec, request_rx: Receiver<OcrRequest>) {
    while let Ok(OcrRequest::Recognize { image, respond }) = request_rx.recv() {
        let _ = respond.send(recognize_on_worker(&mut det, &mut rec, &image));
    }
}

fn recognize_on_worker(
    det: &mut Det,
    rec: &mut Rec,
    img: &image::DynamicImage,
) -> Result<OcrResult, OcrError> {
    let mut rects = det
        .find_text_rect(img)
        .map_err(|e| OcrError::RecognitionFailed(e.to_string()))?;
    // Reading order: top to bottom; boxes on the same line left to right.
    rects.sort_by(|a, b| {
        if (a.top() - b.top()).abs() < LINE_TOLERANCE_PX {
            a.left().cmp(&b.left())
        } else {
            a.top().cmp(&b.top())
        }
    });

    let mut boxes = Vec::with_capacity(rects.len());
    for rect in rects {
        let cropped = crop_clamped(img, rect.left(), rect.top(), rect.width(), rect.height());
        let (text, confidence) = rec
            .predict_with_confidence(&cropped)
            .map_err(|e| OcrError::RecognitionFailed(e.to_string()))?;
        if text.trim().is_empty() {
            continue;
        }
        boxes.push(BoundingBox {
            x: rect.left() as f32,
            y: rect.top() as f32,
            width: rect.width() as f32,
            height: rect.height() as f32,
            text,
            confidence,
        });
    }

    let text = boxes.iter().map(|b| b.text.as_str()).collect::<Vec<_>>().join("\n");
    let confidence = if boxes.is_empty() {
        0.0
    } else {
        boxes.iter().map(|b| b.confidence).sum::<f32>() / boxes.len() as f32
    };

    Ok(OcrResult { text, confidence, boxes })
}

/// Downscale proportionally when either dimension exceeds `max_size`.
fn downscale_to(
    img: image::DynamicImage,
    max_size: Option<(u32, u32)>,
) -> image::DynamicImage {
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

/// Crop with coordinates clamped to the image bounds; detection boxes may
/// overshoot by a pixel or two.
fn crop_clamped(
    img: &image::DynamicImage,
    left: i32,
    top: i32,
    width: u32,
    height: u32,
) -> image::DynamicImage {
    let x = left.max(0) as u32;
    let y = top.max(0) as u32;
    let w = width.min(img.width().saturating_sub(x)).max(1);
    let h = height.min(img.height().saturating_sub(y)).max(1);
    img.crop_imm(x, y, w, h)
}

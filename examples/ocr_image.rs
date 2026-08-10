//! OCR a single image file: `cargo run --example ocr_image -- <image> [models-dir]`

use anydoc::ocr::{EmbeddedOcrBackend, OcrBackend, OcrOptions};

fn main() {
    let image_path = std::env::args().nth(1).expect("usage: ocr_image <image> [models-dir]");
    let models = std::env::args().nth(2).unwrap_or_else(|| "models".to_string());

    let backend = EmbeddedOcrBackend::from_model_dir(&models).expect("failed to load models");
    let bytes = std::fs::read(&image_path).expect("failed to read image");

    let result = backend.recognize(&bytes, &OcrOptions::default()).expect("OCR failed");
    println!("confidence: {:.2}", result.confidence);
    println!("{}", result.text);
}

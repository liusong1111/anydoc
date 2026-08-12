//! 采样 OCR 验证：对模糊情况用低分辨率快速验证。
//!
//! 核心思路：
//! - 降采样到 1/4 分辨率（面积变为 1/4，速度提升 4-10 倍）
//! - 快速 OCR 识别
//! - 如果识别出足够多字符且置信度高 → 是文字图片

use image::GenericImageView;

use crate::model::Asset;
use crate::ocr::backend::{OcrBackend, OcrError, OcrOptions};

/// 采样 OCR 配置
#[derive(Debug, Clone)]
pub struct SamplingOcrOptions {
    /// 降采样比例（0.5 = 宽高各缩小到 1/2，面积变为 1/4）
    pub downsample_ratio: f32,
    /// 最小字符数阈值（忽略空格）
    pub min_char_count: usize,
    /// 最小置信度阈值 [0.0, 1.0]
    pub min_confidence: f32,
}

impl Default for SamplingOcrOptions {
    fn default() -> Self {
        Self {
            downsample_ratio: 0.5,
            min_char_count: 20,
            min_confidence: 0.6,
        }
    }
}

/// 快速采样 OCR 验证
///
/// 步骤：
/// 1. 解码图片
/// 2. 降采样到指定比例
/// 3. 重新编码为 PNG（OCR 引擎输入格式）
/// 4. 快速 OCR 识别
/// 5. 判断字符数和置信度
///
/// # 性能
///
/// - 降采样到 1/2：面积变为 1/4，OCR 速度提升 4-10 倍
/// - 典型耗时：50-200ms（vs 全图 OCR 500-2000ms）
pub fn quick_sample_ocr(
    asset: &Asset,
    backend: &dyn OcrBackend,
    options: &SamplingOcrOptions,
) -> Result<bool, OcrError> {
    // 1. 解码图片
    let img = image::load_from_memory(&asset.bytes)
        .map_err(|e| OcrError::InvalidImage(format!("Image decode failed: {}", e)))?;

    let (orig_width, orig_height) = img.dimensions();

    // 2. 计算新尺寸（保证最小 400px，OCR 需要足够分辨率）
    let new_width = ((orig_width as f32 * options.downsample_ratio) as u32).max(400);
    let new_height = ((orig_height as f32 * options.downsample_ratio) as u32).max(400);

    log::debug!(
        "Sampling OCR: {}x{} → {}x{} (ratio {:.2})",
        orig_width,
        orig_height,
        new_width,
        new_height,
        options.downsample_ratio
    );

    // 3. 降采样（Triangle 滤波器：质量和速度平衡）
    let small = image::imageops::resize(
        &img,
        new_width,
        new_height,
        image::imageops::FilterType::Triangle,
    );

    // 4. 编码为 PNG
    let mut buf = Vec::new();
    small
        .write_to(
            &mut std::io::Cursor::new(&mut buf),
            image::ImageFormat::Png,
        )
        .map_err(|e| OcrError::InvalidImage(format!("Image encode failed: {}", e)))?;

    // 5. 快速 OCR
    let ocr_options = OcrOptions::default();
    let result = backend.recognize(&buf, &ocr_options)?;

    // 6. 判断：字符数（忽略空格）和置信度
    let char_count = result
        .text
        .chars()
        .filter(|c| !c.is_whitespace())
        .count();
    let is_text = char_count >= options.min_char_count
        && result.confidence >= options.min_confidence;

    log::debug!(
        "Sampling OCR result: {} chars (min {}), confidence {:.2} (min {:.2}), is_text={}",
        char_count,
        options.min_char_count,
        result.confidence,
        options.min_confidence,
        is_text
    );

    Ok(is_text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocr::backend::OcrResult;

    /// Mock OCR backend for testing
    struct MockOcrBackend {
        result: OcrResult,
    }

    impl OcrBackend for MockOcrBackend {
        fn recognize(
            &self,
            _bytes: &[u8],
            _options: &OcrOptions,
        ) -> Result<OcrResult, OcrError> {
            Ok(self.result.clone())
        }

        fn health_check(&self) -> Result<(), OcrError> {
            Ok(())
        }
    }

    fn create_test_asset(width: u32, height: u32) -> Asset {
        // 创建简单的白色图片
        let img = image::DynamicImage::new_rgb8(width, height);
        let mut buf = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut buf),
            image::ImageFormat::Png,
        )
        .unwrap();

        Asset {
            id: crate::model::AssetId(0),
            media_type: "image/png".to_string(),
            origin_part: String::new(),
            bytes: buf,
        }
    }

    #[test]
    fn test_sampling_passes_threshold() {
        let asset = create_test_asset(1000, 1000);
        let backend = MockOcrBackend {
            result: OcrResult {
                text: "这是一段测试文字，包含超过20个字符用于测试采样OCR功能".to_string(),
                confidence: 0.85,
                boxes: vec![],
            },
        };

        let options = SamplingOcrOptions::default();
        let result = quick_sample_ocr(&asset, &backend, &options).unwrap();

        assert!(result, "应该通过采样验证（字符数和置信度都满足）");
    }

    #[test]
    fn test_sampling_fails_char_count() {
        let asset = create_test_asset(1000, 1000);
        let backend = MockOcrBackend {
            result: OcrResult {
                text: "短文字".to_string(), // < 20 字符
                confidence: 0.85,
                boxes: vec![],
            },
        };

        let options = SamplingOcrOptions::default();
        let result = quick_sample_ocr(&asset, &backend, &options).unwrap();

        assert!(!result, "应该失败（字符数不足）");
    }

    #[test]
    fn test_sampling_fails_confidence() {
        let asset = create_test_asset(1000, 1000);
        let backend = MockOcrBackend {
            result: OcrResult {
                text: "这是一段测试文字，包含超过20个字符但置信度很低".to_string(),
                confidence: 0.3, // < 0.6
                boxes: vec![],
            },
        };

        let options = SamplingOcrOptions::default();
        let result = quick_sample_ocr(&asset, &backend, &options).unwrap();

        assert!(!result, "应该失败（置信度不足）");
    }

    #[test]
    fn test_sampling_respects_min_size() {
        // 小图片应该保持最小 400px
        let asset = create_test_asset(200, 200);
        let backend = MockOcrBackend {
            result: OcrResult {
                text: "测试文字内容足够长以满足字符数要求".to_string(),
                confidence: 0.85,
                boxes: vec![],
            },
        };

        let options = SamplingOcrOptions {
            downsample_ratio: 0.5,
            min_char_count: 20,
            min_confidence: 0.6,
        };

        // 不应该崩溃，最小尺寸保护生效
        let result = quick_sample_ocr(&asset, &backend, &options);
        assert!(result.is_ok());
    }
}

//! HTTP API server mode for the `any2md` binary (feature = "server").
//!
//! Mirrors the any2text service contract: `POST /v2/any2md` with a multipart
//! body carrying either a `file` upload or a `path` field naming a file local
//! to the server. OCR is on by default; pass an `ocr` field with a false-ish
//! value (`false`, `0`, `no`, `off`) to disable it per request. An
//! `Accept: text/plain` header selects a raw Markdown response body; anything
//! else gets the JSON envelope `{"code", "message", "data"}`.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Multipart, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::json;
use tokio::sync::Semaphore;
use tower_http::{cors::CorsLayer, timeout::TimeoutLayer};

use crate::ocr::{EmbeddedOcrBackend, OcrStrategy};
use crate::{ConvertError, Format};

/// Shared server state: the OCR backend (unless started with `--no-ocr`),
/// the strategy for treating embedded images as page scans, and concurrency
/// control via semaphores + metrics.
pub struct AppState {
    ocr: Option<EmbeddedOcrBackend>,
    strategy: OcrStrategy,

    /// Concurrency limit for OCR tasks (CPU-intensive)
    ocr_semaphore: Arc<Semaphore>,

    /// Concurrency limit for non-OCR tasks (I/O-bound, allows more)
    parse_semaphore: Arc<Semaphore>,

    /// Metrics for monitoring
    metrics: Arc<Metrics>,
}

/// Runtime metrics exposed via /metrics endpoint
struct Metrics {
    /// Requests currently being processed
    active_tasks: AtomicUsize,
    /// Requests waiting in queue
    queued_tasks: AtomicUsize,
    /// Total requests processed since server start
    total_processed: AtomicU64,
    /// Total OCR requests processed
    ocr_processed: AtomicU64,
}

impl Metrics {
    fn new() -> Self {
        Self {
            active_tasks: AtomicUsize::new(0),
            queued_tasks: AtomicUsize::new(0),
            total_processed: AtomicU64::new(0),
            ocr_processed: AtomicU64::new(0),
        }
    }
}

impl AppState {
    /// Build the shared state. `ocr` is `None` when the server runs without
    /// an OCR backend; requests asking for OCR are then rejected.
    ///
    /// Concurrency limits:
    /// - OCR tasks: limited to CPU core count (CPU-bound)
    /// - Parse tasks: 2x CPU cores (has I/O waiting time)
    pub fn new(ocr: Option<EmbeddedOcrBackend>, strategy: OcrStrategy) -> Self {
        let cpu_cores = num_cpus::get();

        // Conservative limits: prevent OOM and CPU thrashing
        let ocr_limit = match cpu_cores {
            1..=2 => 1,
            3..=4 => 2,
            5..=8 => 4,
            9..=16 => 8,
            _ => 12,
        };

        let parse_limit = cpu_cores * 2;

        log::info!(
            "concurrency limits: ocr={}, parse={} (cpu_cores={})",
            ocr_limit,
            parse_limit,
            cpu_cores
        );

        Self {
            ocr,
            strategy,
            ocr_semaphore: Arc::new(Semaphore::new(ocr_limit)),
            parse_semaphore: Arc::new(Semaphore::new(parse_limit)),
            metrics: Arc::new(Metrics::new()),
        }
    }
}

/// The API router: `POST /v2/any2md` for conversion, `GET /metrics` for
/// monitoring. Includes a 2 GiB body limit, a 10-minute request timeout
/// (504 on expiry), and permissive CORS.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v2/any2md", post(convert_handler))
        .route("/metrics", get(metrics_handler))
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024 * 1024))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::GATEWAY_TIMEOUT,
            std::time::Duration::from_secs(10 * 60),
        ))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

/// False-ish values for the `ocr` multipart field; anything else (including
/// a missing field) means OCR is on.
fn ocr_disabled(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "false" | "0" | "no" | "off"
    )
}

/// Client errors answer with `code: 400` in the body. In JSON mode the HTTP
/// status stays 200 (the any2text contract: callers branch on `code`);
/// `Accept: text/plain` gets real HTTP status codes and a plain-text message.
fn client_error(json_mode: bool, message: String) -> Response {
    if json_mode {
        (
            StatusCode::OK,
            Json(json!({"code": 400, "message": message})),
        )
            .into_response()
    } else {
        (StatusCode::BAD_REQUEST, message).into_response()
    }
}

fn internal_error(json_mode: bool, message: String) -> Response {
    if json_mode {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"code": 500, "message": message})),
        )
            .into_response()
    } else {
        (StatusCode::INTERNAL_SERVER_ERROR, message).into_response()
    }
}

/// Metrics endpoint: `GET /metrics`
///
/// Returns Prometheus-style metrics for monitoring server health and load.
async fn metrics_handler(State(state): State<Arc<AppState>>) -> Response {
    let m = &state.metrics;

    let active = m.active_tasks.load(Ordering::Relaxed);
    let queued = m.queued_tasks.load(Ordering::Relaxed);
    let total = m.total_processed.load(Ordering::Relaxed);
    let ocr = m.ocr_processed.load(Ordering::Relaxed);

    let ocr_limit = state.ocr_semaphore.available_permits();
    let parse_limit = state.parse_semaphore.available_permits();

    let body = format!(
        "# HELP any2md_active_tasks Number of requests currently being processed\n\
         # TYPE any2md_active_tasks gauge\n\
         any2md_active_tasks {active}\n\
         \n\
         # HELP any2md_queued_tasks Number of requests waiting in queue\n\
         # TYPE any2md_queued_tasks gauge\n\
         any2md_queued_tasks {queued}\n\
         \n\
         # HELP any2md_total_processed Total requests processed since server start\n\
         # TYPE any2md_total_processed counter\n\
         any2md_total_processed {total}\n\
         \n\
         # HELP any2md_ocr_processed Total OCR requests processed\n\
         # TYPE any2md_ocr_processed counter\n\
         any2md_ocr_processed {ocr}\n\
         \n\
         # HELP any2md_ocr_slots_available OCR semaphore available permits\n\
         # TYPE any2md_ocr_slots_available gauge\n\
         any2md_ocr_slots_available {ocr_limit}\n\
         \n\
         # HELP any2md_parse_slots_available Parse semaphore available permits\n\
         # TYPE any2md_parse_slots_available gauge\n\
         any2md_parse_slots_available {parse_limit}\n"
    );

    (StatusCode::OK, body).into_response()
}

async fn convert_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Response {
    let accept = headers.get("Accept").and_then(|it| it.to_str().ok());
    let json_mode = accept != Some("text/plain");

    let mut file_name: Option<String> = None;
    let mut file_data: Option<axum::body::Bytes> = None;
    let mut path_param: Option<String> = None;
    let mut ocr_wanted = true;

    while let Ok(Some(field)) = multipart.next_field().await {
        match field.name().unwrap_or_default() {
            "file" => {
                if let Some(name) = field.file_name() {
                    file_name = Some(name.to_string());
                }
                match field.bytes().await {
                    Ok(bytes) => file_data = Some(bytes),
                    Err(err) => {
                        return client_error(
                            json_mode,
                            format!("没有上传文件内容,error={err}"),
                        );
                    }
                }
            }
            "path" => {
                if let Ok(text) = field.text().await {
                    let text = text.trim().to_string();
                    if !text.is_empty() {
                        path_param = Some(text);
                    }
                }
            }
            "ocr" => {
                if let Ok(text) = field.text().await {
                    ocr_wanted = !ocr_disabled(&text);
                }
            }
            _ => {}
        }
    }

    // In-memory conversion throughout: uploads never touch disk.
    let (filename, bytes) = match (file_name, file_data) {
        (Some(name), Some(data)) => (name, data.to_vec()),
        _ => match path_param {
            Some(path) => {
                let path = Path::new(&path);
                if !path.exists() {
                    return client_error(json_mode, format!("本地文件不存在,path={}", path.display()));
                }
                let filename = path
                    .file_name()
                    .map(|it| it.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.display().to_string());
                match std::fs::read(path) {
                    Ok(bytes) => (filename, bytes),
                    Err(err) => {
                        return internal_error(
                            json_mode,
                            format!("读取文件失败,文件路径={}, error={err}", path.display()),
                        );
                    }
                }
            }
            None => {
                return client_error(
                    json_mode,
                    "需要提供file字段(上传文件)或path字段(本地文件路径)".to_string(),
                );
            }
        },
    };

    let Some(format) = Format::from_bytes(&bytes).or_else(|| Format::from_path(Path::new(&filename)))
    else {
        return client_error(
            json_mode,
            format!("unrecognized file content and extension: {filename}"),
        );
    };
    log::info!("convert {filename} ({format:?}, ocr={ocr_wanted})");

    // ===== Concurrency control: acquire semaphore permit =====
    state.metrics.queued_tasks.fetch_add(1, Ordering::Relaxed);

    let semaphore = if ocr_wanted && state.ocr.is_some() {
        &state.ocr_semaphore
    } else {
        &state.parse_semaphore
    };

    let _permit = match semaphore.acquire().await {
        Ok(p) => p,
        Err(_) => {
            state.metrics.queued_tasks.fetch_sub(1, Ordering::Relaxed);
            return internal_error(json_mode, "server semaphore closed".to_string());
        }
    };

    state.metrics.queued_tasks.fetch_sub(1, Ordering::Relaxed);
    state.metrics.active_tasks.fetch_add(1, Ordering::Relaxed);
    // =========================================================

    let state2 = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        match (ocr_wanted, state2.ocr.as_ref()) {
            (true, None) => Err(ConvertError::Unsupported(
                "OCR requested but the server was started with --no-ocr".into(),
            )),
            (true, Some(backend)) => {
                crate::to_markdown_bytes_with_ocr(&bytes, format, Some(backend), state2.strategy)
            }
            (false, _) => crate::to_markdown_bytes(&bytes, format),
        }
    })
    .await;

    // Release metrics
    state.metrics.active_tasks.fetch_sub(1, Ordering::Relaxed);
    state.metrics.total_processed.fetch_add(1, Ordering::Relaxed);
    if ocr_wanted {
        state.metrics.ocr_processed.fetch_add(1, Ordering::Relaxed);
    }
    // _permit dropped here, releasing semaphore slot

    let markdown = match result {
        Ok(Ok(markdown)) => markdown,
        Ok(Err(err)) => {
            return client_error(json_mode, format!("解析文件失败,文件={filename}, error={err}"));
        }
        Err(err) => {
            return internal_error(json_mode, format!("转换任务失败,文件={filename}, error={err}"));
        }
    };

    if !json_mode {
        return (StatusCode::OK, markdown).into_response();
    }
    (
        StatusCode::OK,
        Json(json!({
            "code": 200,
            "message": "ok",
            "data": {
                "file": filename,
                "full_text": markdown,
                "ocr": ocr_wanted,
            },
        })),
    )
        .into_response()
}

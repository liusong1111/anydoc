//! HTTP API tests for the `server` feature: drive the router in-process via
//! `tower::ServiceExt::oneshot`, with no OCR backend (model files are not
//! part of the repo).

use std::sync::Arc;

use anydoc::ocr::OcrStrategy;
use anydoc::server::{AppState, router};
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

fn app() -> axum::Router {
    router(Arc::new(AppState::new(None, OcrStrategy::Conservative)))
}

/// One multipart body part carrying a text field.
fn text_part(boundary: &str, name: &str, value: &str) -> String {
    format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
    )
}

fn multipart_request(parts: Vec<String>, accept: Option<&str>) -> Request<Body> {
    let boundary = "TESTBOUNDARY";
    let mut body: Vec<u8> = parts.join("").into_bytes();
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    let mut builder = Request::post("/v2/any2md").header(
        "Content-Type",
        format!("multipart/form-data; boundary={boundary}"),
    );
    if let Some(accept) = accept {
        builder = builder.header("Accept", accept);
    }
    builder.body(Body::from(body)).unwrap()
}

fn file_part(name: &str, filename: &str, data: &[u8]) -> String {
    String::from_utf8(
        format!("--TESTBOUNDARY\r\nContent-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\nContent-Type: application/octet-stream\r\n\r\n")
            .into_bytes()
            .into_iter()
            .chain(data.iter().copied())
            .chain(b"\r\n".iter().copied())
            .collect(),
    )
    .unwrap()
}

async fn response(response: axum::response::Response) -> (StatusCode, serde_json::Value, String) {
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8_lossy(&bytes).to_string();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json, text)
}

#[tokio::test]
async fn missing_file_and_path_is_a_client_error() {
    let (status, json, _) =
        response(app().oneshot(multipart_request(vec![], None)).await.unwrap()).await;
    // JSON mode: HTTP 200 with the failure carried by `code` (any2text contract).
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["code"], 400);
}

#[tokio::test]
async fn nonexistent_local_path_is_a_client_error() {
    let req = multipart_request(
        vec![text_part("TESTBOUNDARY", "path", "/no/such/file.pdf")],
        None,
    );
    let (status, json, _) = response(app().oneshot(req).await.unwrap()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["code"], 400);
}

#[tokio::test]
async fn upload_converts_with_ocr_disabled() {
    let csv = std::fs::read("tests/fixtures/csv/sheet.csv").unwrap();
    let req = multipart_request(
        vec![
            file_part("file", "sheet.csv", &csv),
            text_part("TESTBOUNDARY", "ocr", "false"),
        ],
        None,
    );
    let (status, json, _) = response(app().oneshot(req).await.unwrap()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["code"], 200);
    assert_eq!(json["data"]["file"], "sheet.csv");
    assert_eq!(json["data"]["ocr"], false);
    let markdown = json["data"]["full_text"].as_str().unwrap();
    assert!(markdown.contains("Percent"), "markdown: {markdown}");
}

#[tokio::test]
async fn accept_text_plain_returns_raw_markdown() {
    let csv = std::fs::read("tests/fixtures/csv/sheet.csv").unwrap();
    let req = multipart_request(
        vec![
            file_part("file", "sheet.csv", &csv),
            text_part("TESTBOUNDARY", "ocr", "false"),
        ],
        Some("text/plain"),
    );
    let (status, _, text) = response(app().oneshot(req).await.unwrap()).await;
    assert_eq!(status, StatusCode::OK);
    assert!(text.contains("Percent"), "body: {text}");
    assert!(!text.contains("\"code\""), "body: {text}");
}

#[tokio::test]
async fn ocr_defaults_on_and_is_rejected_without_a_backend() {
    let csv = std::fs::read("tests/fixtures/csv/sheet.csv").unwrap();
    let req = multipart_request(vec![file_part("file", "sheet.csv", &csv)], None);
    let (status, json, _) = response(app().oneshot(req).await.unwrap()).await;
    // The router in these tests has no OCR backend (--no-ocr state), so the
    // default-on OCR request must be rejected, not silently downgraded.
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["code"], 400);
    assert!(json["message"].as_str().unwrap().contains("--no-ocr"));
}

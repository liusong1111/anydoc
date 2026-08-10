//! `any2md server` subcommand: load the OCR backend and serve the HTTP API
//! (the router itself lives in `anydoc::server` so tests can reach it).

use std::sync::Arc;

use anydoc::ConvertError;
use anydoc::ocr::EmbeddedOcrBackend;

use crate::{Cli, ServerArgs};

pub async fn run(cli: &Cli, args: &ServerArgs) -> Result<(), ConvertError> {
    let ocr = if args.no_ocr {
        log::warn!("started with --no-ocr; requests asking for OCR will be rejected");
        None
    } else {
        Some(
            EmbeddedOcrBackend::from_model_dir_with_threads(&cli.ocr_models, cli.ocr_threads)
                .map_err(|e| {
                    ConvertError::Unsupported(format!(
                        "{e} (model files missing? run scripts/download-models.sh, or pass --no-ocr)"
                    ))
                })?,
        )
    };
    let state = Arc::new(anydoc::server::AppState::new(ocr, cli.strategy()));

    let addr = format!("0.0.0.0:{}", args.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    log::info!("listening on {addr}");
    axum::serve(listener, anydoc::server::router(state)).await?;
    Ok(())
}

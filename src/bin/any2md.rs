//! any2md — convert documents to Markdown, with optional OCR:
//! `any2md <file> [-o out.md] [-f csv] [--ocr] [--ocr-strategy aggressive]`

use std::path::PathBuf;
use std::process::ExitCode;

use anydoc::ocr::{EmbeddedOcrBackend, OcrStrategy};
use anydoc::{ConvertError, Format};
use clap::Parser;

#[derive(Parser)]
#[command(name = "any2md", about = "Convert documents to Markdown, with optional OCR")]
struct Cli {
    /// Input document.
    #[arg(value_name = "INPUT")]
    input: PathBuf,

    /// Write the Markdown here instead of stdout.
    #[arg(short, long, value_name = "OUTPUT")]
    output: Option<PathBuf>,

    /// Name the input format (e.g. csv) instead of detecting it.
    #[arg(short, long, value_name = "FORMAT")]
    format: Option<String>,

    /// Enable OCR for scanned PDF pages and embedded page scans.
    #[arg(long)]
    ocr: bool,

    /// How eagerly embedded images are treated as page scans.
    #[arg(long, default_value = "conservative", value_parser = ["conservative", "aggressive"])]
    ocr_strategy: String,

    /// Directory holding the PP-OCRv5-FP16 model files.
    #[arg(long, default_value = "models", value_name = "DIR")]
    ocr_models: PathBuf,

    /// OCR inference thread count (default: engine decides).
    #[arg(long, value_name = "N")]
    ocr_threads: Option<u32>,

    /// Only print the detected format; do not convert.
    #[arg(long)]
    detect: bool,

    /// Increase log verbosity (-v info, -vv debug, -vvv trace).
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    env_logger::Builder::new()
        .filter_level(match cli.verbose {
            0 => log::LevelFilter::Warn,
            1 => log::LevelFilter::Info,
            2 => log::LevelFilter::Debug,
            _ => log::LevelFilter::Trace,
        })
        .init();

    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> Result<(), ConvertError> {
    let bytes = std::fs::read(&cli.input)?;
    // Without -f the format comes from the file content, with the extension
    // as the fallback for signature-less formats (CSV).
    let format = match cli
        .format
        .as_deref()
        .and_then(Format::from_extension)
        .or_else(|| Format::from_bytes(&bytes))
        .or_else(|| Format::from_path(&cli.input))
    {
        Some(format) => format,
        None if cli.format.is_some() => {
            return Err(ConvertError::Unsupported(format!(
                "unknown format: {}",
                cli.format.as_deref().unwrap_or_default()
            )));
        }
        None => {
            return Err(ConvertError::Unsupported(format!(
                "unrecognized file content and extension: {}",
                cli.input.display()
            )));
        }
    };

    if cli.detect {
        println!("{format:?}");
        return Ok(());
    }
    log::info!("detected format: {format:?}");

    let markdown = if cli.ocr {
        let backend =
            EmbeddedOcrBackend::from_model_dir_with_threads(&cli.ocr_models, cli.ocr_threads)
                .map_err(|e| {
                    ConvertError::Unsupported(format!(
                        "{e} (model files missing? run scripts/download-models.sh)"
                    ))
                })?;
        let strategy = match cli.ocr_strategy.as_str() {
            "conservative" => OcrStrategy::Conservative,
            // clap's value_parser restricts the input, so this is the only
            // other reachable value.
            _ => OcrStrategy::Aggressive,
        };
        anydoc::to_markdown_bytes_with_ocr(&bytes, format, Some(&backend), strategy)?
    } else {
        anydoc::to_markdown_bytes(&bytes, format)?
    };

    match &cli.output {
        Some(path) => std::fs::write(path, markdown)?,
        None => {
            use std::io::Write;
            let _ = std::io::stdout().write_all(markdown.as_bytes());
        }
    }
    Ok(())
}

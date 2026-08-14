//! any2md — convert documents to Markdown, with optional OCR:
//! `any2md <file> [-o out.md] [-f csv] [--ocr] [--ocr-strategy aggressive]`
//! `any2md server [--port 8766] [--no-ocr]`

mod server;

use std::path::PathBuf;
use std::process::ExitCode;

use anydoc::ocr::{EmbeddedOcrBackend, OcrStrategy};
use anydoc::{ConvertError, Format, OutputFormat};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "any2md", about = "Convert documents to Markdown, with optional OCR")]
struct Cli {
    /// Input document.
    #[arg(value_name = "INPUT")]
    input: Option<PathBuf>,

    /// Write the Markdown here instead of stdout.
    #[arg(short, long, value_name = "OUTPUT")]
    output: Option<PathBuf>,

    /// Export embedded illustrations to this directory (requires -o/--output).
    #[arg(long, value_name = "DIR")]
    images_dir: Option<PathBuf>,

    /// Name the input format (e.g. csv) instead of detecting it.
    #[arg(short, long, value_name = "FORMAT")]
    format: Option<String>,

    /// Output format: markdown (default), plain text, or HTML.
    #[arg(short = 'F', long, default_value = "markdown", value_parser = ["markdown", "md", "plain", "text", "plaintext", "txt", "html", "htm", "html-full", "html-doc"])]
    output_format: String,

    /// Enable OCR for scanned PDF pages and embedded page scans.
    #[arg(long)]
    ocr: bool,

    /// How eagerly embedded images are treated as page scans.
    #[arg(long, default_value = "smart", value_parser = ["disabled", "conservative", "smart", "aggressive"])]
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

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run as an HTTP API server (POST /v2/any2md).
    Server(ServerArgs),
}

#[derive(clap::Args)]
struct ServerArgs {
    /// Port to listen on.
    #[arg(short, long, default_value = "8766")]
    port: u16,

    /// Start without an OCR backend; requests asking for OCR are rejected.
    #[arg(long)]
    no_ocr: bool,
}

impl Cli {
    fn strategy(&self) -> OcrStrategy {
        match self.ocr_strategy.as_str() {
            "disabled" => OcrStrategy::Disabled,
            "conservative" => OcrStrategy::Conservative,
            "smart" => OcrStrategy::Smart,
            "aggressive" => OcrStrategy::Aggressive,
            _ => OcrStrategy::Smart, // 默认 Smart
        }
    }

    fn output_format(&self) -> OutputFormat {
        self.output_format.parse().unwrap_or_default()
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    let mut logger = env_logger::Builder::new();
    logger.filter_level(match cli.verbose {
        0 => log::LevelFilter::Warn,
        1 => log::LevelFilter::Info,
        2 => log::LevelFilter::Debug,
        _ => log::LevelFilter::Trace,
    });
    // lopdf dumps entire font dictionaries at Warn when it cannot parse an
    // encoding (common with Chrome/Skia Type3 fonts). The standard-encoding
    // fallback it then applies works fine, so only surface these with -v.
    if cli.verbose == 0 {
        logger.filter_module("lopdf", log::LevelFilter::Error);
    }
    logger.init();

    let result = match &cli.command {
        Some(Command::Server(args)) => server::run(&cli, args).await,
        None => run(&cli),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> Result<(), ConvertError> {
    let Some(input) = &cli.input else {
        return Err(ConvertError::Unsupported(
            "missing input document (or a subcommand such as `server`)".into(),
        ));
    };
    if cli.images_dir.is_some() && cli.output.is_none() {
        return Err(ConvertError::Unsupported(
            "--images-dir requires -o/--output to write files next to".into(),
        ));
    }
    let bytes = std::fs::read(input)?;
    // Without -f the format comes from the file content, with the extension
    // as the fallback for signature-less formats (CSV).
    let format = match cli
        .format
        .as_deref()
        .and_then(Format::from_extension)
        .or_else(|| Format::from_bytes(&bytes))
        .or_else(|| Format::from_path(input))
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
                input.display()
            )));
        }
    };

    if cli.detect {
        println!("{format:?}");
        return Ok(());
    }
    log::info!("detected format: {format:?}");

    let image_prefix = cli.images_dir.as_ref().map(|d| d.to_string_lossy().into_owned());
    let output = if cli.ocr {
        let backend =
            EmbeddedOcrBackend::from_model_dir_with_threads(&cli.ocr_models, cli.ocr_threads)
                .map_err(|e| {
                    ConvertError::Unsupported(format!(
                        "{e} (model files missing? run scripts/download-models.sh)"
                    ))
                })?;
        anydoc::to_output_with_ocr(
            &bytes,
            format,
            cli.output_format(),
            Some(&backend),
            cli.strategy(),
            image_prefix.as_deref(),
        )?
    } else {
        anydoc::to_output_with_ocr(
            &bytes,
            format,
            cli.output_format(),
            None,
            OcrStrategy::Disabled,
            image_prefix.as_deref(),
        )?
    };

    match &cli.output {
        Some(path) => {
            std::fs::write(path, &output.content)?;
            if let Some(dir) = &cli.images_dir {
                write_images(path, dir, &output.images)?;
            }
        }
        None => {
            use std::io::Write;
            let _ = std::io::stdout().write_all(output.content.as_bytes());
        }
    }
    Ok(())
}

fn write_images(
    output_path: &std::path::Path,
    images_dir: &std::path::Path,
    images: &[anydoc::ExportedImage],
) -> Result<(), ConvertError> {
    let base = output_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(images_dir);
    std::fs::create_dir_all(&base)?;
    for image in images {
        std::fs::write(base.join(&image.filename), &image.bytes)?;
    }
    Ok(())
}

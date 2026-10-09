//! What the binary accepts, and what it refuses to start without.
//!
//! Every knob is a flag with an environment variable behind it. Secrets arrive by file
//! or by environment and never as a flag, because an argument is visible in
//! `kubectl describe pod` and in every process listing in the container.

use std::{net::SocketAddr, path::PathBuf};

use clap::{Args as ClapArgs, Parser, Subcommand};

use crate::StartupError;

#[derive(Debug, Parser)]
#[command(version, about = "Pre-printed QR stickers, and what they open")]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,

    /// Parse and validate everything, then exit without doing any of it.
    ///
    /// Global, so it can be appended to whatever the Helm chart renders. The chart and
    /// this binary are separate artifacts with no compiler between them, and this is
    /// what lets CI check that the flags one writes are the flags the other accepts.
    #[arg(long, global = true)]
    pub check_config: bool,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Serve the web service.
    Serve(Box<ServeArgs>),
    /// Apply any migrations the database is missing, then exit.
    Migrate(DatabaseArgs),
    /// Render a sheet of codes, or a calibration page, to a PDF.
    Sheet(Box<SheetArgs>),
    /// Show what each code length and error-correction level costs in QR modules.
    Budget(BudgetArgs),
    /// Delete blobs nothing refers to any more.
    Gc(GcArgs),
}

#[derive(Debug, Clone, ClapArgs)]
pub struct DatabaseArgs {
    /// A file holding the connection string.
    ///
    /// What a deployment uses: the cluster mounts the database secret's `uri` key and
    /// points this at it, so the password never becomes an argument.
    #[arg(long, env = "QR_DATABASE_URL_FILE", conflicts_with = "database_url")]
    pub database_url_file: Option<PathBuf>,

    /// The connection string itself. For a development shell.
    #[arg(long, env = "QR_DATABASE_URL", hide_env_values = true)]
    pub database_url: Option<String>,

    /// A one-instance Postgres allows a hundred; this leaves room for psql and a job.
    #[arg(long, env = "QR_DATABASE_MAX_CONNECTIONS", default_value_t = 8)]
    pub database_max_connections: u32,
}

impl DatabaseArgs {
    /// Resolves the connection string, reading the file if that is where it lives.
    pub fn url(&self) -> Result<String, StartupError> {
        if let Some(path) = &self.database_url_file {
            let raw =
                std::fs::read_to_string(path).map_err(|source| StartupError::DatabaseUrlFile {
                    path: path.clone(),
                    source,
                })?;
            return Ok(raw.trim().to_owned());
        }
        self.database_url.clone().ok_or(StartupError::NoDatabase)
    }
}

#[derive(Debug, Clone, ClapArgs)]
pub struct CodeArgs {
    /// Characters in a code id.
    ///
    /// Longer costs nothing at the default secret length: a QR version holds a range of
    /// lengths, and every id this accepts lands inside one. See `budget`.
    #[arg(long, env = "QR_CODE_ID_LEN", default_value_t = qr_core::ids::DEFAULT_CODE_ID_LEN)]
    pub code_id_len: usize,

    /// Characters in a capability secret.
    #[arg(long, env = "QR_CODE_SECRET_LEN", default_value_t = qr_core::ids::DEFAULT_SECRET_LEN)]
    pub code_secret_len: usize,

    /// QR error-correction level: L, M, Q or H. The only real lever on symbol size.
    #[arg(long, env = "QR_EC_LEVEL", default_value = "Q")]
    pub qr_ec_level: String,
}

#[derive(Debug, Clone, ClapArgs)]
pub struct TypstArgs {
    /// The typst binary. The Nix wrapper puts the flake's own on PATH ahead of any other.
    #[arg(long, env = "QR_TYPST_BIN", default_value = "typst")]
    pub typst_bin: PathBuf,

    /// Extra fonts. Rendering uses only typst's embedded faces otherwise, so that a
    /// laptop with a font tree and a container with none produce the same sheet.
    #[arg(long, env = "QR_TYPST_FONT_PATH")]
    pub typst_font_path: Option<PathBuf>,
}

#[derive(Debug, ClapArgs)]
pub struct ServeArgs {
    #[command(flatten)]
    pub database: DatabaseArgs,

    #[command(flatten)]
    pub codes: CodeArgs,

    #[command(flatten)]
    pub typst: TypstArgs,

    /// Apply missing migrations at startup. One replica, so nothing races.
    #[arg(
        long,
        env = "QR_AUTO_MIGRATE",
        default_value_t = true,
        action = clap::ArgAction::Set,
    )]
    pub auto_migrate: bool,

    #[arg(long, env = "QR_BIND", default_value = "0.0.0.0:8080")]
    pub bind: SocketAddr,

    /// Where the probes answer. A second port on purpose: /readyz queries the database
    /// and is not something to route from the internet.
    #[arg(long, env = "QR_HEALTH_BIND", default_value = "0.0.0.0:9090")]
    pub health_bind: SocketAddr,

    /// The public name, encoded into every sticker this service prints.
    ///
    /// Never an in-cluster address: a printed sticker outlives any Service.
    #[arg(
        long,
        env = "QR_PUBLIC_BASE_URL",
        default_value = "https://qr.shine.town"
    )]
    pub base_url: String,

    /// Where photo blobs live. A mounted volume in a deployment.
    #[arg(long, env = "QR_DATA_DIR", default_value = "/data")]
    pub blob_root: PathBuf,

    #[arg(long, env = "QR_MAX_UPLOAD_BYTES", default_value_t = 26_214_400)]
    pub max_upload_bytes: usize,

    /// The guard that matters: a two-megabyte file can declare fifty thousand pixels a
    /// side and ask for a four-gigabyte allocation before anything is decoded.
    #[arg(long, env = "QR_MAX_UPLOAD_PIXELS", default_value_t = 50_000_000)]
    pub max_upload_pixels: u32,

    #[arg(long, env = "QR_COOKIE_MAX_AGE_DAYS", default_value_t = 365)]
    pub cookie_max_age_days: i64,

    /// Re-encode uploads to strip metadata, including wherever the photo was taken.
    #[arg(long, env = "QR_STRIP_ORIGINAL_EXIF")]
    pub strip_original_exif: bool,

    /// Drop Secure from the capability cookie, so plain-HTTP localhost works.
    #[arg(long, env = "QR_INSECURE_COOKIES")]
    pub insecure_cookies: bool,

    /// Replace the built-in layouts, for stock these do not cover.
    #[arg(long, env = "QR_PRESETS_FILE")]
    pub presets_file: Option<PathBuf>,

    /// Sheets rendered at once.
    ///
    /// Each one forks typst, which peaks around 31MB resident and holds a core for about
    /// a tenth of a second. Two at a time leaves the pod's memory ceiling to the photo
    /// uploads that arrive all year round rather than to the printing done twice a year.
    #[arg(long, env = "QR_SHEET_CONCURRENCY", default_value_t = 2)]
    pub sheet_concurrency: u8,
}

#[derive(Debug, ClapArgs)]
pub struct SheetArgs {
    #[command(flatten)]
    pub database: DatabaseArgs,

    #[command(flatten)]
    pub codes: CodeArgs,

    #[command(flatten)]
    pub typst: TypstArgs,

    #[arg(
        long,
        env = "QR_PUBLIC_BASE_URL",
        default_value = "https://qr.shine.town"
    )]
    pub base_url: String,

    /// How many codes. Defaults to filling one sheet of the chosen stock.
    #[arg(long)]
    pub count: Option<usize>,

    #[arg(long, default_value = "relief-t7160")]
    pub preset: String,

    /// Replace the built-in layouts, for stock these do not cover.
    #[arg(long, env = "QR_PRESETS_FILE")]
    pub presets_file: Option<PathBuf>,

    /// What every code on this sheet becomes on first scan. Omitted leaves the chooser.
    #[arg(long)]
    pub handler: Option<String>,

    /// Printed small at the foot, so two printings can be told apart later.
    #[arg(long)]
    pub note: Option<String>,

    #[arg(long, short, default_value = "sheet.pdf")]
    pub out: PathBuf,

    /// Print crosshairs and rulers instead of codes, to measure a printer's own page
    /// origin against. Touches no database.
    #[arg(long)]
    pub calibration: bool,
}

#[derive(Debug, ClapArgs)]
pub struct BudgetArgs {
    #[arg(
        long,
        env = "QR_PUBLIC_BASE_URL",
        default_value = "https://qr.shine.town"
    )]
    pub base_url: String,

    /// Id lengths to tabulate.
    #[arg(long, value_delimiter = ',', default_values_t = [6usize, 8, 10, 12])]
    pub id_lens: Vec<usize>,

    /// Secret lengths to tabulate.
    #[arg(long, value_delimiter = ',', default_values_t = [12usize, 16, 20, 24])]
    pub secret_lens: Vec<usize>,

    /// The printed width to report millimetres per module against.
    #[arg(long, default_value_t = 27.0)]
    pub qr_mm: f32,
}

#[derive(Debug, ClapArgs)]
pub struct GcArgs {
    #[command(flatten)]
    pub database: DatabaseArgs,

    #[arg(long, env = "QR_DATA_DIR", default_value = "/data")]
    pub blob_root: PathBuf,

    /// Leave anything newer than this alone. A blob is written before its row commits,
    /// so a cutoff is what keeps the collector from racing an upload.
    #[arg(long, default_value = "7d")]
    pub older_than: String,
}

/// Starts tracing on stderr.
///
/// `QR_LOG` wins over `RUST_LOG`, so turning this service up does not also turn up
/// everything else sharing a shell.
pub fn init_tracing() {
    use tracing_subscriber::EnvFilter;

    let filter = EnvFilter::try_from_env("QR_LOG")
        .or_else(|_| EnvFilter::try_from_default_env())
        .unwrap_or_else(|_| EnvFilter::new("qr_server=info,qr_core=info,tower_http=warn"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}

/// Prints an error and everything underneath it.
///
/// By hand rather than through a reporting crate, so the chain is on stderr in the shape
/// a container log reads well in: one line, then one indented line per cause.
pub fn report(err: &dyn std::error::Error) {
    eprintln!("error: {err}");
    let mut source = err.source();
    while let Some(cause) = source {
        eprintln!("  caused by: {cause}");
        source = cause.source();
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use clap::CommandFactory as _;

    fn parse(argv: &[&str]) -> Result<Args, clap::Error> {
        Args::try_parse_from(std::iter::once("qr-server").chain(argv.iter().copied()))
    }

    #[test]
    fn the_command_line_is_internally_consistent() {
        Args::command().debug_assert();
    }

    #[test]
    fn check_config_is_accepted_after_every_subcommand() {
        // What the chart check relies on: whatever the manifest renders, plus this.
        for argv in [
            vec!["serve", "--bind=0.0.0.0:8080", "--check-config"],
            vec!["migrate", "--check-config"],
            vec!["sheet", "--calibration", "--check-config"],
            vec!["budget", "--check-config"],
            vec!["gc", "--check-config"],
        ] {
            let args = parse(&argv).unwrap_or_else(|e| panic!("{argv:?}: {e}"));
            assert!(args.check_config);
        }
    }

    #[test]
    fn the_arguments_the_chart_renders_all_parse() {
        let args = parse(&[
            "serve",
            "--bind=0.0.0.0:8080",
            "--health-bind=0.0.0.0:9090",
            "--base-url=https://qr.shine.town",
            "--blob-root=/data",
            "--code-id-len=10",
            "--code-secret-len=16",
            "--qr-ec-level=Q",
            "--max-upload-bytes=26214400",
            "--max-upload-pixels=50000000",
        ])
        .expect("the chart's arguments should parse");
        assert!(!args.check_config);
    }

    #[test]
    fn giving_both_a_database_url_and_a_file_is_refused() {
        let err = parse(&[
            "migrate",
            "--database-url=postgres://a/b",
            "--database-url-file=/run/db",
        ])
        .expect_err("two sources should conflict");
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn a_missing_database_url_says_where_to_put_one() {
        let args = DatabaseArgs {
            database_url_file: None,
            database_url: None,
            database_max_connections: 8,
        };
        let err = args.url().expect_err("no source");
        let message = err.to_string();
        assert!(message.contains("QR_DATABASE_URL"), "{message}");
    }

    #[test]
    fn a_database_url_read_from_a_file_loses_its_trailing_newline() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("uri");
        std::fs::write(&path, "postgres://qr@localhost/qr\n").expect("write");

        let args = DatabaseArgs {
            database_url_file: Some(path),
            database_url: None,
            database_max_connections: 8,
        };
        assert_eq!(args.url().expect("url"), "postgres://qr@localhost/qr");
    }
}

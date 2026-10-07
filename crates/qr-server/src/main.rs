//! The qr.shine.town server.

use std::process::ExitCode;

use clap::Parser as _;
use qr_server::cli::{self, Args};

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    cli::init_tracing();

    match qr_server::run(args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            cli::report(&err);
            // Distinct from 1, so a supervisor can tell a refused start from a crash.
            ExitCode::from(2)
        }
    }
}

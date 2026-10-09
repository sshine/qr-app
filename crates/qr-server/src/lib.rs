//! The qr.shine.town service.
//!
//! A library with a thin `main.rs` over it, so an integration test can start the real
//! server in-process and drive it without a subprocess or a socket.

pub mod app;
pub mod assets;
pub mod cli;
pub mod error;
pub mod health;
pub mod sheet;
pub mod shutdown;

pub use error::StartupError;

use std::sync::Arc;

use qr_core::{BaseUrl, BlobStore, FsBlobStore, Store};
use sqlx::postgres::PgPoolOptions;

use cli::{Args, Command, DatabaseArgs, ServeArgs};

/// Runs whatever the command line asked for.
pub async fn run(args: Args) -> Result<(), StartupError> {
    match args.command {
        Command::Serve(serve) => {
            let serve = *serve;
            validate_serve(&serve)?;
            if args.check_config {
                return Ok(());
            }
            self::serve(serve).await
        }
        Command::Migrate(database) => {
            if args.check_config {
                return Ok(());
            }
            let store = connect(&database).await?;
            store.migrate().await.map_err(StartupError::Migrate)
        }
        Command::Sheet(sheet) => sheet::run(*sheet, args.check_config).await,
        Command::Budget(budget) => sheet::budget(&budget),
        Command::Gc(gc) => {
            if args.check_config {
                return Ok(());
            }
            let _ = connect(&gc.database).await?;
            // The collector lands with the handler that creates blobs to collect.
            tracing::warn!("gc is not implemented yet; nothing was deleted");
            Ok(())
        }
    }
}

/// Everything that can be checked without opening a socket or a connection.
///
/// Separate from `serve` so `--check-config` exercises exactly this and nothing else.
fn validate_serve(args: &ServeArgs) -> Result<(), StartupError> {
    // Refused here rather than discovered on the day a sheet is printed: a base URL that
    // cannot encode densely makes every sticker a bigger symbol than the layout expects.
    let base = BaseUrl::parse(&args.base_url).map_err(StartupError::BaseUrl)?;

    let ec = qr_sheet::parse_ec_level(&args.codes.qr_ec_level)?;
    let budget = qr_sheet::budget(
        &base,
        args.codes.code_id_len,
        args.codes.code_secret_len,
        ec,
    )?;
    tracing::debug!(
        version = budget.version,
        modules = budget.modules,
        "redeem urls encode at this size"
    );

    if args.insecure_cookies && !args.bind.ip().is_loopback() {
        return Err(StartupError::InsecureCookiesOffLoopback { addr: args.bind });
    }

    sheet::check_presets(args.presets_file.as_deref())?;

    Ok(())
}

async fn connect(args: &DatabaseArgs) -> Result<Store, StartupError> {
    let url = args.url()?;
    let pool = PgPoolOptions::new()
        .max_connections(args.database_max_connections)
        .connect(&url)
        .await
        .map_err(StartupError::Database)?;
    Ok(Store::new(pool))
}

async fn serve(args: ServeArgs) -> Result<(), StartupError> {
    let store = connect(&args.database).await?;
    if args.auto_migrate {
        store.migrate().await.map_err(StartupError::Migrate)?;
    }

    let blobs: Arc<dyn BlobStore> =
        Arc::new(FsBlobStore::open(&args.blob_root).await.map_err(|source| {
            StartupError::BlobRoot {
                path: args.blob_root.clone(),
                source,
            }
        })?);

    // Both listeners come up together and go down together. A process serving requests
    // whose probes have stopped answering is worse than one that is simply gone.
    let registry = app::registry().map_err(|e| StartupError::Registry(e.to_string()))?;
    let state = app::state(&args, store.clone(), Arc::clone(&blobs), registry).await?;

    let health = health::Health::new(store.clone(), Arc::clone(&blobs)).router();
    let public = app::router(state.clone(), sheet::routes::state(&args, state)?);

    let public_listener = listen(args.bind).await?;
    let health_listener = listen(args.health_bind).await?;

    tracing::info!(public = %args.bind, health = %args.health_bind, "listening");

    let public = axum::serve(public_listener, public).with_graceful_shutdown(shutdown::signal());
    let health = axum::serve(health_listener, health).with_graceful_shutdown(shutdown::signal());

    let (public, health) = tokio::join!(public, health);
    public.map_err(StartupError::Serve)?;
    health.map_err(StartupError::Serve)?;

    store.pool().close().await;
    Ok(())
}

async fn listen(addr: std::net::SocketAddr) -> Result<tokio::net::TcpListener, StartupError> {
    tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|source| StartupError::Bind { addr, source })
}

//! When to stop.

/// Resolves on SIGTERM or Ctrl-C.
///
/// SIGTERM is what Kubernetes sends; a server listening only for Ctrl-C is killed some
/// seconds later with whatever was in flight half-written.
pub async fn signal() {
    let interrupt = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut stream) => {
                stream.recv().await;
            }
            // Nothing can be done about a handler that will not install, and exiting
            // here would turn a missing signal into no service at all.
            Err(e) => {
                tracing::error!(error = %e, "could not listen for SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = interrupt => tracing::info!("interrupted"),
        () = terminate => tracing::info!("terminated"),
    }
}

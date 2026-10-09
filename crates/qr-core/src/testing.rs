//! A database per test.
//!
//! Per-database rather than a transaction rolled back at the end: the code under test
//! opens transactions of its own, and wrapping it in one makes the atomicity of a claim
//! or a bind untestable, which is most of what there is to get wrong here.
//!
//! Behind the `testing` feature so the CREATE DATABASE and DROP DATABASE statements
//! below are not linked into anything that only reads and writes rows.

use sqlx::{AssertSqlSafe, Connection as _, PgConnection, PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

use crate::{Error, Result, Store};

/// Where the tests find a server.
///
/// `QR_TEST_DATABASE_URL` rather than `DATABASE_URL`, so pointing a shell at a database
/// with real codes in it cannot be what a test drops.
pub const URL_VAR: &str = "QR_TEST_DATABASE_URL";

/// A scratch database, dropped when this goes out of scope.
pub struct TestDb {
    server_url: String,
    name: String,
    pool: Option<PgPool>,
}

impl TestDb {
    /// Creates a database, migrates it, and hands back a pool onto it.
    ///
    /// Panics with instructions rather than skipping when the variable is unset. A
    /// suite that quietly runs nothing is indistinguishable from one that passes.
    pub async fn create() -> Result<Self> {
        let server_url = std::env::var(URL_VAR).unwrap_or_else(|_| {
            panic!(
                "{URL_VAR} is not set, so the tests that need a database cannot run.\n\
                 Locally:  just db-up && {URL_VAR}=\"$DATABASE_URL\" just test-db\n\
                 In Nix:   nix flake check, which starts one in the sandbox."
            )
        });

        // A database name cannot be a bind parameter, so it is interpolated. It is a
        // UUID this line just generated and never anything a caller supplied, which is
        // what AssertSqlSafe below is asserting; the check states that rather than
        // leaving a reader to work it out.
        let name = format!("qr_test_{}", Uuid::now_v7().simple());
        debug_assert!(name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'));

        let create = format!(r#"CREATE DATABASE "{name}""#);
        let mut server = PgConnection::connect(&server_url).await?;
        sqlx::raw_sql(AssertSqlSafe(create))
            .execute(&mut server)
            .await?;
        server.close().await?;

        // A handful of connections, not sqlx's default ten. The suite runs its tests in
        // parallel and each one takes a database of its own, so the default multiplied
        // by the number of tests walks straight into Postgres' max_connections and the
        // whole run hangs waiting for a slot rather than failing.
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .min_connections(0)
            .connect(&swap_database(&server_url, &name)?)
            .await?;
        Store::new(pool.clone()).migrate().await?;

        Ok(Self {
            server_url,
            name,
            pool: Some(pool),
        })
    }

    pub fn pool(&self) -> &PgPool {
        // Only `drop` clears this, and nothing can call a method on a dropped value.
        #[expect(
            clippy::expect_used,
            reason = "the option is Some for the whole life of the value; only Drop takes it"
        )]
        self.pool.as_ref().expect("pool")
    }

    pub fn store(&self) -> Store {
        Store::new(self.pool().clone())
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        let Some(pool) = self.pool.take() else {
            return;
        };

        // Dropped rather than closed. The pool's sockets are registered with the reactor
        // of the runtime the test ran on, and awaiting close() from the runtime built
        // below would wait on a reactor nothing polls again, which hangs the whole suite.
        // DROP DATABASE ... WITH (FORCE) evicts whatever backends are left instead.
        drop(pool);

        let server_url = self.server_url.clone();
        let name = self.name.clone();

        // A thread with a runtime of its own, because Drop is not async and the test's
        // runtime is already winding down around it. The connection this opens is new,
        // so it belongs to this runtime and can be driven here.
        let _ = std::thread::spawn(move || {
            let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            rt.block_on(async move {
                let drop_db = format!(r#"DROP DATABASE IF EXISTS "{name}" WITH (FORCE)"#);
                if let Ok(mut server) = PgConnection::connect(&server_url).await {
                    let _ = sqlx::raw_sql(AssertSqlSafe(drop_db))
                        .execute(&mut server)
                        .await;
                    let _ = server.close().await;
                }
            });
        })
        .join();
    }
}

/// Replaces the database name in a connection URL, keeping everything else.
fn swap_database(url: &str, name: &str) -> Result<String> {
    let mut parsed =
        url::Url::parse(url).map_err(|e| Error::Invalid(format!("{URL_VAR} is not a URL: {e}")))?;
    parsed.set_path(name);
    Ok(parsed.into())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    #[test]
    fn swapping_the_database_keeps_the_rest_of_the_url() {
        let swapped =
            swap_database("postgres://qr:qr@127.0.0.1:5433/qr", "qr_test_1").expect("swap");
        assert_eq!(swapped, "postgres://qr:qr@127.0.0.1:5433/qr_test_1");
    }

    #[test]
    fn swapping_works_against_a_unix_socket_url() {
        // What the Nix sandbox hands over: no host, a socket directory in the query.
        let swapped = swap_database(
            "postgres://postgres@localhost/postgres?host=/tmp/sock",
            "qr_test_2",
        )
        .expect("swap");
        assert_eq!(
            swapped,
            "postgres://postgres@localhost/qr_test_2?host=/tmp/sock"
        );
    }
}

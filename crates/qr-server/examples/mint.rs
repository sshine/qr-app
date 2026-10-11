//! Mints codes into the configured database and prints them as JSON.
//!
//! A fixture tool for the end-to-end suite, which needs a code together with its
//! plaintext secret and has nowhere else to get one: a printed sheet carries the pair
//! only inside a PDF, and every other path hashes the secret before it is reachable.
//!
//! An example rather than a binary, so it is not installed and cannot be reached in a
//! deployment. It mints real codes, so point it at a throwaway database.

use qr_core::{MintedCode, Store, ids};
use serde_json::json;
use sqlx::postgres::PgPoolOptions;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::var("QR_DATABASE_URL")?;
    let count: usize = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "1".to_owned())
        .parse()?;

    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await?;
    let store = Store::new(pool);
    let mut conn = store.pool().acquire().await?;

    let minted = qr_core::store::mint_codes(
        &mut conn,
        count,
        ids::DEFAULT_CODE_ID_LEN,
        ids::DEFAULT_SECRET_LEN,
        None,
        None,
        &json!({}),
    )
    .await?;

    let codes: Vec<_> = minted
        .iter()
        .map(|MintedCode { id, secret }| json!({ "id": id.to_string(), "secret": secret.expose() }))
        .collect();

    println!("{}", serde_json::to_string_pretty(&codes)?);
    Ok(())
}

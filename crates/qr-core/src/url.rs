//! The URLs a sticker carries.
//!
//! A redeem URL is uppercase on purpose. QR's alphanumeric mode covers digits, capitals
//! and a handful of punctuation, and nothing else; a lowercase character forces the whole
//! segment into byte mode and costs roughly three characters' worth of capacity each.
//! Browsers lowercase the scheme and the host themselves, so nothing is lost by shouting.
//!
//! The capability secret rides in the fragment, which a browser never sends. It therefore
//! appears in no access log, no `Referer` and no proxy along the way. The cost is that
//! redeeming needs a line of script to read `location.hash`; the typed form on the sticker
//! is what covers a visitor without it.

use crate::{CodeId, Error, Result, Secret};

/// Everything QR alphanumeric mode can encode.
const QR_ALPHANUMERIC: &str = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ $%*+-./:";

/// True when every character of `s` survives QR's alphanumeric mode.
pub fn is_qr_alphanumeric(s: &str) -> bool {
    s.chars().all(|c| QR_ALPHANUMERIC.contains(c))
}

/// Where this deployment answers, in the form a sticker encodes.
///
/// Validated once at startup rather than per sheet, so a base URL that cannot be encoded
/// densely is a refused start and not a surprise on the one day a sheet is printed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseUrl {
    /// Canonical, for links a human reads.
    canonical: String,
    /// Uppercased, for what goes inside a QR.
    shouted: String,
}

impl BaseUrl {
    pub fn parse(raw: &str) -> Result<Self> {
        let parsed =
            url::Url::parse(raw).map_err(|e| Error::Invalid(format!("{raw} is not a URL: {e}")))?;

        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(Error::Invalid(format!(
                "{raw} must be http or https, not {}",
                parsed.scheme()
            )));
        }
        if parsed.query().is_some() || parsed.fragment().is_some() {
            return Err(Error::Invalid(format!(
                "{raw} must carry no query and no fragment; the fragment is where the \
                 capability secret goes"
            )));
        }

        let canonical = parsed.as_str().trim_end_matches('/').to_owned();
        let shouted = canonical.to_uppercase();

        if !is_qr_alphanumeric(&shouted) {
            return Err(Error::Invalid(format!(
                "{canonical} contains characters outside QR alphanumeric mode even \
                 uppercased, which would push every sticker into byte mode"
            )));
        }

        Ok(Self { canonical, shouted })
    }

    /// The form a human or a browser sees.
    pub fn as_str(&self) -> &str {
        &self.canonical
    }

    /// What a QR encodes: uppercase, with the secret in the fragment.
    pub fn redeem_url(&self, id: &CodeId, secret: &Secret) -> String {
        format!("{}/C/{}#{}", self.shouted, id, secret.expose())
    }

    /// The page a redeemed code lands on, with no secret in it.
    pub fn code_url(&self, id: &CodeId) -> String {
        format!("{}/c/{}", self.canonical, id)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use crate::ids::{DEFAULT_CODE_ID_LEN, DEFAULT_SECRET_LEN};

    fn base() -> BaseUrl {
        BaseUrl::parse("https://qr.shine.town").expect("parse")
    }

    #[test]
    fn a_redeem_url_is_entirely_qr_alphanumeric_but_for_the_fragment_marker() {
        let base = base();
        for _ in 0..10_000 {
            let id = CodeId::generate(DEFAULT_CODE_ID_LEN).expect("id");
            let secret = Secret::generate(DEFAULT_SECRET_LEN).expect("secret");
            let url = base.redeem_url(&id, &secret);
            let without_marker = url.replace('#', "");
            assert!(
                is_qr_alphanumeric(&without_marker),
                "{url} would not encode in alphanumeric mode"
            );
        }
    }

    #[test]
    fn a_redeem_url_is_the_length_the_encoding_budget_assumes() {
        let base = base();
        let id = CodeId::generate(DEFAULT_CODE_ID_LEN).expect("id");
        let secret = Secret::generate(DEFAULT_SECRET_LEN).expect("secret");
        // 21 for the host and scheme, 3 for /C/, 8 for the id, 1 for #, 16 for the secret.
        assert_eq!(base.redeem_url(&id, &secret).len(), 49);
    }

    #[test]
    fn the_shouted_form_is_what_the_sticker_carries() {
        let base = base();
        let id = CodeId::parse("ABC12345").expect("id");
        let secret = Secret::parse("H7TQ4R9WX2ND5PKC").expect("secret");
        assert_eq!(
            base.redeem_url(&id, &secret),
            "HTTPS://QR.SHINE.TOWN/C/ABC12345#H7TQ4R9WX2ND5PKC"
        );
        assert_eq!(base.code_url(&id), "https://qr.shine.town/c/ABC12345");
    }

    #[test]
    fn a_trailing_slash_does_not_become_a_double_slash() {
        let base = BaseUrl::parse("https://qr.shine.town/").expect("parse");
        let id = CodeId::parse("ABC12345").expect("id");
        assert_eq!(base.code_url(&id), "https://qr.shine.town/c/ABC12345");
    }

    #[test]
    fn a_base_url_that_cannot_be_encoded_densely_is_refused() {
        // An underscore survives neither uppercasing nor alphanumeric mode.
        assert!(BaseUrl::parse("https://qr_test.shine.town").is_err());
    }

    #[test]
    fn a_base_url_carrying_a_fragment_is_refused() {
        assert!(BaseUrl::parse("https://qr.shine.town/#x").is_err());
        assert!(BaseUrl::parse("https://qr.shine.town/?a=b").is_err());
    }

    #[test]
    fn a_base_url_that_is_not_http_is_refused() {
        assert!(BaseUrl::parse("ftp://qr.shine.town").is_err());
        assert!(BaseUrl::parse("qr.shine.town").is_err());
    }
}

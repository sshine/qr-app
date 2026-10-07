//! Code identities and capability secrets.
//!
//! Both are Crockford base32. Every character of that alphabet is also in QR's
//! alphanumeric mode, which is what lets a redeem URL encode as a small symbol, and the
//! letters it leaves out (I, L, O, U) are the ones a human transcribing a sticker would
//! confuse. The decode rules map I and L back to 1 and O back to 0, so a code typed from
//! a sticker resolves whichever way it was read.

use std::fmt;

use subtle::ConstantTimeEq as _;

use crate::{Error, Result};

/// Crockford base32, in value order.
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Domain separation for the capability hash. Changing this invalidates every stored
/// hash, which is a migration, not a tweak.
const SECRET_CONTEXT: &str = "qr.shine.town 2026-10-07 capability secret";

/// The widest id or secret either type will hold. Both are configurable within their own
/// bounds, and the arrays are sized for the maximum so neither allocates.
const MAX_LEN: usize = 24;

pub const CODE_ID_LEN_RANGE: std::ops::RangeInclusive<usize> = 6..=12;
pub const SECRET_LEN_RANGE: std::ops::RangeInclusive<usize> = 12..=24;

/// The default id length, 40 bits. Generation retries on the primary key anyway, so this
/// is about how long an id can be left lying around before a retry is likely, not about
/// whether a collision is survivable.
pub const DEFAULT_CODE_ID_LEN: usize = 8;

/// The default secret length, 80 bits. Uniform randomness, so there is nothing to
/// brute-force offline and no reason for a password KDF over it.
pub const DEFAULT_SECRET_LEN: usize = 16;

/// Decodes one character under Crockford's rules, or returns `None`.
///
/// Case is folded, `I` and `L` read as `1`, `O` reads as `0`. `U` is excluded from the
/// alphabet and is not accepted as anything.
fn decode_char(c: u8) -> Option<u8> {
    match c.to_ascii_uppercase() {
        b'0' | b'O' => Some(0),
        b'1' | b'I' | b'L' => Some(1),
        c @ b'2'..=b'9' => Some(c - b'0'),
        b'A' => Some(10),
        b'B' => Some(11),
        b'C' => Some(12),
        b'D' => Some(13),
        b'E' => Some(14),
        b'F' => Some(15),
        b'G' => Some(16),
        b'H' => Some(17),
        b'J' => Some(18),
        b'K' => Some(19),
        b'M' => Some(20),
        b'N' => Some(21),
        b'P' => Some(22),
        b'Q' => Some(23),
        b'R' => Some(24),
        b'S' => Some(25),
        b'T' => Some(26),
        b'V' => Some(27),
        b'W' => Some(28),
        b'X' => Some(29),
        b'Y' => Some(30),
        b'Z' => Some(31),
        _ => None,
    }
}

/// Fills `out` with uniformly distributed alphabet characters.
///
/// Masking the low five bits is exactly uniform only because 256 is a whole number of
/// alphabet lengths. The assertion is what makes an alphabet that stops being 32 long a
/// compile error rather than a quiet bias nobody measures.
fn random_chars(out: &mut [u8]) -> Result<()> {
    const _: () = assert!(ALPHABET.len() == 32);

    getrandom::fill(out).map_err(|_| Error::Entropy)?;
    for slot in out.iter_mut() {
        *slot = ALPHABET[usize::from(*slot & 0x1f)];
    }
    Ok(())
}

/// Strips the separators Crockford allows in a transcribed string.
fn normalize(raw: &str) -> Vec<u8> {
    raw.bytes()
        .filter(|b| !matches!(b, b'-' | b' '))
        .map(|b| b.to_ascii_uppercase())
        .collect()
}

/// A code id: the characters printed under a QR, and the primary key of a sticker.
///
/// Not secret. Knowing one buys nothing without the capability secret beside it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CodeId {
    chars: [u8; MAX_LEN],
    len: u8,
}

impl CodeId {
    /// Generates an id of `len` characters.
    pub fn generate(len: usize) -> Result<Self> {
        if !CODE_ID_LEN_RANGE.contains(&len) {
            return Err(Error::BadLength {
                what: "code id",
                len,
            });
        }
        let mut chars = [0u8; MAX_LEN];
        random_chars(&mut chars[..len])?;
        Ok(Self {
            chars,
            #[allow(clippy::cast_possible_truncation)]
            len: len as u8,
        })
    }

    /// Parses an id, applying Crockford's transcription rules.
    pub fn parse(raw: &str) -> Result<Self> {
        let bytes = normalize(raw);
        if !CODE_ID_LEN_RANGE.contains(&bytes.len()) {
            return Err(Error::BadLength {
                what: "code id",
                len: bytes.len(),
            });
        }
        let mut chars = [0u8; MAX_LEN];
        for (slot, &b) in chars.iter_mut().zip(bytes.iter()) {
            let v = decode_char(b).ok_or(Error::BadCharacter { what: "code id" })?;
            *slot = ALPHABET[usize::from(v)];
        }
        Ok(Self {
            chars,
            #[allow(clippy::cast_possible_truncation)]
            len: bytes.len() as u8,
        })
    }

    #[expect(
        clippy::expect_used,
        reason = "every byte is copied out of ALPHABET, which is ASCII; proved by \
                  a_generated_id_uses_only_the_alphabet"
    )]
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.chars[..usize::from(self.len)]).expect("alphabet is ascii")
    }

    pub fn len(&self) -> usize {
        usize::from(self.len)
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The id grouped in fours, as it is printed on a sticker.
    pub fn grouped(&self) -> String {
        group(self.as_str())
    }
}

impl fmt::Display for CodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Debug for CodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CodeId({})", self.as_str())
    }
}

impl std::str::FromStr for CodeId {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        Self::parse(s)
    }
}

/// A capability secret: what the QR carries past the code id, and what the cookie holds.
///
/// `Debug` and `Display` are redacted. `expose` is the only way to the characters, so a
/// leak into a log is a deliberate call rather than a derive nobody reread.
#[derive(Clone)]
pub struct Secret {
    chars: [u8; MAX_LEN],
    len: u8,
}

impl Secret {
    pub fn generate(len: usize) -> Result<Self> {
        if !SECRET_LEN_RANGE.contains(&len) {
            return Err(Error::BadLength {
                what: "secret",
                len,
            });
        }
        let mut chars = [0u8; MAX_LEN];
        random_chars(&mut chars[..len])?;
        Ok(Self {
            chars,
            #[allow(clippy::cast_possible_truncation)]
            len: len as u8,
        })
    }

    pub fn parse(raw: &str) -> Result<Self> {
        let bytes = normalize(raw);
        if !SECRET_LEN_RANGE.contains(&bytes.len()) {
            return Err(Error::BadLength {
                what: "secret",
                len: bytes.len(),
            });
        }
        let mut chars = [0u8; MAX_LEN];
        for (slot, &b) in chars.iter_mut().zip(bytes.iter()) {
            let v = decode_char(b).ok_or(Error::BadCharacter { what: "secret" })?;
            *slot = ALPHABET[usize::from(v)];
        }
        Ok(Self {
            chars,
            #[allow(clippy::cast_possible_truncation)]
            len: bytes.len() as u8,
        })
    }

    #[expect(
        clippy::expect_used,
        reason = "every byte is copied out of ALPHABET, which is ASCII; proved by \
                  a_generated_secret_uses_only_the_alphabet"
    )]
    pub fn expose(&self) -> &str {
        std::str::from_utf8(&self.chars[..usize::from(self.len)]).expect("alphabet is ascii")
    }

    pub fn len(&self) -> usize {
        usize::from(self.len)
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Grouped in fours, for printing on a sticker as the typed fallback.
    pub fn grouped(&self) -> String {
        group(self.expose())
    }

    /// What is stored. BLAKE3 key derivation rather than a password KDF: the input is
    /// uniform randomness, so there is no dictionary to slow down, and argon2 would cost
    /// tens of milliseconds on every request that carries a cookie.
    pub fn hash(&self) -> SecretHash {
        SecretHash(blake3::derive_key(SECRET_CONTEXT, self.expose().as_bytes()))
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(redacted)")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

impl std::str::FromStr for Secret {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        Self::parse(s)
    }
}

/// The stored form of a capability secret.
#[derive(Clone, Copy)]
pub struct SecretHash([u8; 32]);

impl SecretHash {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Constant time, so a near-miss does not take measurably longer than a wild guess.
    pub fn verify(&self, candidate: &Secret) -> bool {
        self.0.ct_eq(&candidate.hash().0).into()
    }
}

impl fmt::Debug for SecretHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretHash(..)")
    }
}

/// The hash a lookup miss is compared against.
///
/// A miss must spend the same work a hit does, or the response time says whether an id
/// exists and the id space becomes an enumerable inventory of how many boxes there are.
pub fn decoy_hash() -> SecretHash {
    SecretHash(blake3::derive_key(SECRET_CONTEXT, b"decoy"))
}

/// Every character of the alphabet, for callers that need to state the set.
pub fn alphabet() -> &'static str {
    // A literal rather than a conversion, so the accessor cannot fail. The two
    // spellings are checked against each other by a test.
    "0123456789ABCDEFGHJKMNPQRSTVWXYZ"
}

fn group(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + s.len() / 4);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && i % 4 == 0 {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    #[test]
    fn the_two_spellings_of_the_alphabet_agree() {
        assert_eq!(alphabet().as_bytes(), ALPHABET);
    }

    #[test]
    fn a_generated_secret_uses_only_the_alphabet() {
        for _ in 0..256 {
            let secret = Secret::generate(DEFAULT_SECRET_LEN).expect("generate");
            for c in secret.expose().chars() {
                assert!(alphabet().contains(c), "{c} is not in the alphabet");
            }
        }
    }

    #[test]
    fn the_alphabet_leaves_out_the_letters_a_human_would_confuse() {
        for c in ['I', 'L', 'O', 'U'] {
            assert!(!alphabet().contains(c), "{c} should not be in the alphabet");
        }
        assert_eq!(alphabet().len(), 32);
    }

    #[test]
    fn every_alphabet_character_is_in_qr_alphanumeric_mode() {
        // QR alphanumeric mode covers 0-9, A-Z, space and $%*+-./: and nothing else.
        for c in alphabet().chars() {
            assert!(
                c.is_ascii_digit() || c.is_ascii_uppercase(),
                "{c} is outside QR alphanumeric mode"
            );
        }
    }

    #[test]
    fn a_generated_id_round_trips_through_parse() {
        for _ in 0..256 {
            let id = CodeId::generate(DEFAULT_CODE_ID_LEN).expect("generate");
            let again = CodeId::parse(id.as_str()).expect("parse");
            assert_eq!(id, again);
            assert_eq!(id.len(), DEFAULT_CODE_ID_LEN);
        }
    }

    #[test]
    fn a_generated_id_uses_only_the_alphabet() {
        for _ in 0..256 {
            let id = CodeId::generate(12).expect("generate");
            for c in id.as_str().chars() {
                assert!(alphabet().contains(c), "{c} is not in the alphabet");
            }
        }
    }

    #[test]
    fn transcription_folds_the_characters_crockford_says_to_fold() {
        let canonical = CodeId::parse("10101010").expect("canonical");
        for spelling in [
            "IOIOIOIO",
            "lolololo",
            "LoLoLoLo",
            "10-10-10-10",
            "1010 1010",
        ] {
            assert_eq!(
                CodeId::parse(spelling).expect(spelling),
                canonical,
                "{spelling} should read as the canonical form"
            );
        }
    }

    #[test]
    fn u_is_not_accepted_as_anything() {
        assert!(CodeId::parse("UUUUUUUU").is_err());
    }

    #[test]
    fn an_id_of_the_wrong_length_is_refused() {
        assert!(CodeId::parse("ABC12").is_err());
        assert!(CodeId::parse("ABC1234567890").is_err());
        assert!(CodeId::generate(4).is_err());
        assert!(CodeId::generate(16).is_err());
    }

    #[test]
    fn a_character_outside_the_alphabet_is_refused() {
        assert!(CodeId::parse("ABC123!8").is_err());
        assert!(Secret::parse("ABC123!89ABCDEF").is_err());
    }

    #[test]
    fn debug_on_a_secret_does_not_print_it() {
        let secret = Secret::generate(DEFAULT_SECRET_LEN).expect("generate");
        let rendered = format!("{secret:?} {secret}");
        assert!(!rendered.contains(secret.expose()));
        assert!(rendered.contains("redacted"));
    }

    #[test]
    fn a_hash_accepts_its_own_secret_and_rejects_a_neighbour() {
        let secret = Secret::parse("H7TQ4R9WX2ND5PKC").expect("parse");
        let hash = secret.hash();
        assert!(hash.verify(&secret));

        let neighbour = Secret::parse("H7TQ4R9WX2ND5PKD").expect("parse");
        assert!(!hash.verify(&neighbour));
    }

    #[test]
    fn a_hash_is_stable_across_transcriptions_of_one_secret() {
        let typed = Secret::parse("h7tq-4r9w-x2nd-5pkc").expect("parse");
        let scanned = Secret::parse("H7TQ4R9WX2ND5PKC").expect("parse");
        assert_eq!(typed.hash().as_bytes(), scanned.hash().as_bytes());
    }

    #[test]
    fn the_decoy_hash_matches_no_real_secret() {
        let decoy = decoy_hash();
        for _ in 0..64 {
            let secret = Secret::generate(DEFAULT_SECRET_LEN).expect("generate");
            assert!(!decoy.verify(&secret));
        }
    }

    #[test]
    fn printing_groups_in_fours() {
        let id = CodeId::parse("ABC12345").expect("parse");
        assert_eq!(id.grouped(), "ABC1 2345");
        let secret = Secret::parse("H7TQ4R9WX2ND5PKC").expect("parse");
        assert_eq!(secret.grouped(), "H7TQ 4R9W X2ND 5PKC");
    }

    #[test]
    fn generation_is_spread_over_the_whole_alphabet() {
        // A modulo bias or a truncated alphabet would show up as characters that never
        // appear; 32 symbols over 20k draws leaves a vanishing chance of a true miss.
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..2000 {
            let secret = Secret::generate(DEFAULT_SECRET_LEN).expect("generate");
            seen.extend(secret.expose().chars());
        }
        assert_eq!(
            seen.len(),
            32,
            "some alphabet characters are never generated"
        );
    }
}

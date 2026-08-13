//! Content fingerprints.
//!
//! blake3, and **only at admission and at read time**. Trame never hashes the
//! whole tree: that would pay a cost proportional to the repository for
//! information that concerns a handful of files.

use std::fmt;

use serde::{Deserialize, Serialize};

/// The blake3 fingerprint of a file's contents.
///
/// Serialised as hex, which keeps the SQLite journal readable by eye — that
/// matters for a tool whose main argument is auditability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContentHash(#[serde(with = "hex_bytes")] [u8; 32]);

impl ContentHash {
    /// Hash some contents.
    #[must_use]
    pub fn of(bytes: impl AsRef<[u8]>) -> Self {
        Self(*blake3::hash(bytes.as_ref()).as_bytes())
    }

    /// The raw bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// The full hex form, 64 characters.
    #[must_use]
    pub fn to_hex(&self) -> String {
        self.0
            .iter()
            .fold(String::with_capacity(64), |mut acc, byte| {
                use fmt::Write as _;
                let _ = write!(acc, "{byte:02x}");
                acc
            })
    }

    /// The first eight hex characters. For display only, never for comparison.
    #[must_use]
    pub fn short(&self) -> String {
        self.to_hex().chars().take(8).collect()
    }

    /// Read a fingerprint back from its hex form.
    ///
    /// This is the journal's return path: the `hash`, `hash_before` and `hash_after`
    /// columns are hex `TEXT`.
    pub fn from_hex(hex: &str) -> Result<Self, InvalidHash> {
        if hex.len() != 64 {
            return Err(InvalidHash);
        }
        let mut out = [0_u8; 32];
        for (index, slot) in out.iter_mut().enumerate() {
            let pair = hex.get(index * 2..index * 2 + 2).ok_or(InvalidHash)?;
            *slot = u8::from_str_radix(pair, 16).map_err(|_| InvalidHash)?;
        }
        Ok(Self(out))
    }
}

/// A string that is not a 64-character hex blake3 fingerprint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid fingerprint: 64 hex characters expected")]
pub struct InvalidHash;

impl std::str::FromStr for ContentHash {
    type Err = InvalidHash;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        Self::from_hex(raw)
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// Hex serialisation of the 32 bytes.
mod hex_bytes {
    use serde::de::{Error as _, Unexpected};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub(super) fn serialize<S: Serializer>(bytes: &[u8; 32], ser: S) -> Result<S::Ok, S::Error> {
        super::ContentHash(*bytes).to_hex().serialize(ser)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<[u8; 32], D::Error> {
        let hex = String::deserialize(de)?;
        // A single decoding implementation, shared with `ContentHash::from_hex`.
        super::ContentHash::from_hex(&hex)
            .map(|hash| *hash.as_bytes())
            .map_err(|_| D::Error::invalid_value(Unexpected::Str(&hex), &"64 hex characters"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_different_contents_hash_differently() {
        assert_ne!(
            ContentHash::of("fn verify_token()"),
            ContentHash::of("fn validate_token()")
        );
    }

    #[test]
    fn the_same_content_always_hashes_the_same() {
        assert_eq!(ContentHash::of(b"auth.rs"), ContentHash::of(b"auth.rs"));
    }

    #[test]
    fn hex_encoding_round_trips() {
        let hash = ContentHash::of("mod auth;");
        let json = serde_json::to_string(&hash).unwrap();
        assert_eq!(json, format!("\"{}\"", hash.to_hex()));
        assert_eq!(serde_json::from_str::<ContentHash>(&json).unwrap(), hash);
    }
}

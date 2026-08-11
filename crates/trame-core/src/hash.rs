//! Empreintes de contenu.
//!
//! blake3, et **uniquement a l'admission et a la lecture**. Trame ne hashe
//! jamais l'arbre entier : ce serait payer un cout proportionnel au depot pour
//! une information qui ne concerne qu'une poignee de fichiers.

use std::fmt;

use serde::{Deserialize, Serialize};

/// L'empreinte blake3 du contenu d'un fichier.
///
/// Serialisee en hexadecimal : le journal SQLite reste lisible a l'oeil nu, ce
/// qui compte pour un outil dont l'argument principal est l'auditabilite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContentHash(#[serde(with = "hex_bytes")] [u8; 32]);

impl ContentHash {
    /// Hashe un contenu.
    #[must_use]
    pub fn of(bytes: impl AsRef<[u8]>) -> Self {
        Self(*blake3::hash(bytes.as_ref()).as_bytes())
    }

    /// Les octets bruts.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// La forme hexadecimale complete, 64 caracteres.
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

    /// Les huit premiers caracteres hexadecimaux. Pour l'affichage seulement,
    /// jamais pour une comparaison.
    #[must_use]
    pub fn short(&self) -> String {
        self.to_hex().chars().take(8).collect()
    }

    /// Relit une empreinte depuis sa forme hexadecimale.
    ///
    /// C'est le chemin de retour du journal : les colonnes `hash`, `hash_before` et
    /// `hash_after` sont du `TEXT` hexadecimal.
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

/// Une chaine qui n'est pas une empreinte blake3 hexadecimale de 64 caracteres.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("empreinte invalide : 64 caracteres hexadecimaux attendus")]
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

/// Serialisation hexadecimale des 32 octets.
mod hex_bytes {
    use serde::de::{Error as _, Unexpected};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub(super) fn serialize<S: Serializer>(bytes: &[u8; 32], ser: S) -> Result<S::Ok, S::Error> {
        super::ContentHash(*bytes).to_hex().serialize(ser)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<[u8; 32], D::Error> {
        let hex = String::deserialize(de)?;
        // Une seule implementation du decodage, partagee avec `ContentHash::from_hex`.
        super::ContentHash::from_hex(&hex)
            .map(|hash| *hash.as_bytes())
            .map_err(|_| {
                D::Error::invalid_value(Unexpected::Str(&hex), &"64 caracteres hexadecimaux")
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deux_contenus_differents_ont_deux_empreintes_differentes() {
        assert_ne!(
            ContentHash::of("fn verify_token()"),
            ContentHash::of("fn validate_token()")
        );
    }

    #[test]
    fn l_empreinte_est_stable() {
        assert_eq!(ContentHash::of(b"auth.rs"), ContentHash::of(b"auth.rs"));
    }

    #[test]
    fn l_hexadecimal_fait_un_aller_retour() {
        let hash = ContentHash::of("mod auth;");
        let json = serde_json::to_string(&hash).unwrap();
        assert_eq!(json, format!("\"{}\"", hash.to_hex()));
        assert_eq!(serde_json::from_str::<ContentHash>(&json).unwrap(), hash);
    }
}

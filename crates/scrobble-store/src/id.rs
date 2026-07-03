//! Stable scrobble identity.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Stable identity for a scrobble: its Unix timestamp plus a short hash of the current
/// `(artist, track)` pair, formatted as `"{uts}-{hex8}"`.
///
/// Properties and consequences, by design:
/// - Two scrobbles at different seconds are always distinct.
/// - Same-second scrobbles of *different* tracks are distinct (hash differs).
/// - Same-second duplicate scrobbles of the *same* artist+track collapse into one record.
///   Ordinal suffixes were rejected because ordinals are unstable across fetches.
/// - An edit that changes artist or track name changes the identity; mirroring such an edit
///   writes a new record and tombstones the old id.
///
/// The hash is over the exact strings (case-sensitive), separated by a NUL byte so that
/// `("ab", "c")` and `("a", "bc")` cannot collide.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ScrobbleId(String);

impl ScrobbleId {
    /// Construct the id for a scrobble observed with the given timestamp and current metadata.
    pub fn new(uts: u64, artist: &str, track: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(artist.as_bytes());
        hasher.update([0u8]);
        hasher.update(track.as_bytes());
        let digest = hasher.finalize();
        let hex8: String = digest[..4].iter().map(|b| format!("{b:02x}")).collect();
        Self(format!("{uts}-{hex8}"))
    }

    /// The scrobble's Unix timestamp, parsed back out of the id.
    pub fn uts(&self) -> u64 {
        self.0
            .split('-')
            .next()
            .and_then(|s| s.parse().ok())
            .expect("ScrobbleId is always constructed with a leading uts")
    }

    /// The id as a string slice (the on-disk representation).
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::str::FromStr for ScrobbleId {
    type Err = String;

    /// Parse the on-disk `"{uts}-{hex8}"` representation.
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let (uts, hex) = raw
            .split_once('-')
            .ok_or_else(|| format!("'{raw}' is not of the form <uts>-<hex8>"))?;
        uts.parse::<u64>()
            .map_err(|_| format!("'{uts}' is not a unix timestamp"))?;
        if hex.len() != 8 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("'{hex}' is not an 8-char hex hash"));
        }
        Ok(Self(raw.to_string()))
    }
}

impl std::fmt::Display for ScrobbleId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::fmt::Debug for ScrobbleId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ScrobbleId({})", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_inputs_same_id() {
        let a = ScrobbleId::new(1_700_000_000, "Boards of Canada", "Roygbiv");
        let b = ScrobbleId::new(1_700_000_000, "Boards of Canada", "Roygbiv");
        assert_eq!(a, b);
    }

    #[test]
    fn different_track_same_second_differs() {
        let a = ScrobbleId::new(1_700_000_000, "Boards of Canada", "Roygbiv");
        let b = ScrobbleId::new(1_700_000_000, "Boards of Canada", "Telephasic Workshop");
        assert_ne!(a, b);
    }

    #[test]
    fn field_boundaries_do_not_collide() {
        let a = ScrobbleId::new(1, "ab", "c");
        let b = ScrobbleId::new(1, "a", "bc");
        assert_ne!(a, b);
    }

    #[test]
    fn uts_round_trips() {
        let id = ScrobbleId::new(1_700_000_123, "x", "y");
        assert_eq!(id.uts(), 1_700_000_123);
    }

    #[test]
    fn from_str_round_trips_and_validates() {
        use std::str::FromStr;
        let id = ScrobbleId::new(1_700_000_000, "a", "b");
        assert_eq!(ScrobbleId::from_str(id.as_str()).unwrap(), id);
        assert!(ScrobbleId::from_str("nonsense").is_err());
        assert!(ScrobbleId::from_str("123-zzzzzzzz").is_err());
    }

    #[test]
    fn serde_is_transparent_string() {
        let id = ScrobbleId::new(5, "a", "b");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, format!("\"{id}\""));
        let back: ScrobbleId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }
}

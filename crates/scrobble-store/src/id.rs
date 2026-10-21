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
#[path = "id_tests.rs"]
mod tests;

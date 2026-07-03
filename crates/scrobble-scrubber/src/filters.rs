//! MusicBrainz release-filter vocabulary, referenced per-rule by
//! [`RewriteRule::musicbrainz_release_filters`](crate::rewrite::RewriteRule).
//!
//! These types are unconditional (not feature-gated) so rule JSON containing filters always
//! parses; only the MusicBrainz *verification* that consumes them is behind the
//! `musicbrainz` feature.

use serde::{Deserialize, Serialize};

/// A single release-filtering criterion for MusicBrainz verification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ReleaseFilterType {
    /// Exclude demo releases
    ExcludeDemo,
    /// Exclude special editions (deluxe, legacy, expanded, etc.)
    ExcludeSpecialEdition,
    /// Prefer non-Japanese releases when multiple are available
    PreferNonJapanese,
    /// Exclude releases with specific disambiguation terms
    ExcludeByDisambiguation { terms: Vec<String> },
    /// Exclude releases from specific countries
    ExcludeByCountry { countries: Vec<String> },
}

/// Release filter configuration for MusicBrainz verification.
///
/// These filters apply only to confirmation/verification of rewritten metadata — never to
/// search operations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReleaseFilterConfig {
    /// List of active filters to apply
    pub filters: Vec<ReleaseFilterType>,
    /// General preference for original releases over reissues
    pub prefer_original_releases: bool,
    /// Additional custom terms to exclude from disambiguation
    pub custom_exclusion_terms: Vec<String>,
}

impl Default for ReleaseFilterConfig {
    fn default() -> Self {
        Self {
            filters: vec![
                ReleaseFilterType::ExcludeDemo,
                ReleaseFilterType::PreferNonJapanese,
                ReleaseFilterType::ExcludeSpecialEdition,
            ],
            prefer_original_releases: true,
            custom_exclusion_terms: Vec::new(),
        }
    }
}

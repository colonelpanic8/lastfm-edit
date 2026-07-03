//! MusicBrainz-backed providers (feature = `musicbrainz`).
//!
//! Ported from the original scrobble-scrubber. Two providers live here:
//! - [`MusicBrainzScrubActionProvider`]: suggests metadata corrections by matching tracks
//!   against the MusicBrainz database, and powers per-rule MusicBrainz confirmation for
//!   rewrite rules.
//! - [`CompilationToCanonicalProvider`]: suggests moving tracks from compilation albums to
//!   the canonical (original studio) release.

pub mod client;
pub mod compilation_provider;
pub mod musicbrainz_provider;

pub use client::{MusicBrainzClient, MusicBrainzMatch};
pub use compilation_provider::{
    default_release_comparer, CompilationToCanonicalProvider, RankedRelease, ReleaseComparer,
};
pub use musicbrainz_provider::MusicBrainzScrubActionProvider;

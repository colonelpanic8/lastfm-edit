//! MusicBrainz-backed scrobble identity.

// Nothing drives these modules until the resolving lane and planning provider exist.
#![allow(dead_code, unused_imports)]

mod canonical;
mod mb;
mod outcome;
mod title;

pub use canonical::{AlbumMove, Canonical, Rung};
pub use mb::{ArtistCredit, ReleaseGroup, UserAgent};
pub use outcome::{MoveMode, MovePolicy, Outcome, Proof, Target};

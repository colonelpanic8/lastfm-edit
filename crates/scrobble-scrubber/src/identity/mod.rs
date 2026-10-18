//! MusicBrainz-backed scrobble identity.

// Nothing drives these modules until the resolving lane and planning provider exist.
#![allow(dead_code, unused_imports)]

mod mb;

pub use mb::{ArtistCredit, ReleaseGroup, UserAgent};

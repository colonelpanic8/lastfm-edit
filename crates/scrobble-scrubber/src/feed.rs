//! Feeds: the different ways a scrub can be driven.
//!
//! A [`ScrubFeed`] names *what to consider*; resolution against the store turns it into
//! ordered [`FeedBatch`]es of records. Only the coverage-driven feeds (`StoreRange`,
//! `Incremental`) advance planning coverage; spot feeds (`Artist`/`Album`/`Ids`) never do.

use scrobble_store::{ScrobbleId, ScrobbleRecord};
use serde::{Deserialize, Serialize};
use std::ops::Range;

/// What to scrub. Serializable so feeds can appear in CLI args, config, and events.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "feed", rename_all = "snake_case")]
pub enum ScrubFeed {
    /// Everything in the store, or a time slice of it.
    StoreRange { range: Option<Range<u64>> },
    /// Work not yet planned: (sync coverage ∩ optional window) − planning coverage.
    Incremental { window: Option<Range<u64>> },
    /// All live scrobbles of one artist (optionally windowed).
    Artist {
        name: String,
        range: Option<Range<u64>>,
    },
    /// One album by one artist.
    Album { artist: String, album: String },
    /// Explicit scrobbles (search results, CLI, UI selection).
    Ids(Vec<ScrobbleId>),
}

impl std::fmt::Display for ScrubFeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScrubFeed::StoreRange { range: None } => write!(f, "store"),
            ScrubFeed::StoreRange { range: Some(r) } => write!(f, "store[{}..{}]", r.start, r.end),
            ScrubFeed::Incremental { window: None } => write!(f, "incremental"),
            ScrubFeed::Incremental { window: Some(w) } => {
                write!(f, "incremental[{}..{}]", w.start, w.end)
            }
            ScrubFeed::Artist { name, .. } => write!(f, "artist:{name}"),
            ScrubFeed::Album { artist, album } => write!(f, "album:{artist}/{album}"),
            ScrubFeed::Ids(ids) => write!(f, "ids({})", ids.len()),
        }
    }
}

/// One resolved batch of work.
#[derive(Clone, Debug)]
pub struct FeedBatch {
    pub records: Vec<ScrobbleRecord>,
    /// Range to insert into planning coverage once this batch's suggestions are durably
    /// recorded. `None` for spot feeds — they never advance coverage.
    pub coverage_claim: Option<Range<u64>>,
}

use crate::error::Result;
use scrobble_store::{CoverageMap, Storage};

impl ScrubFeed {
    /// Resolve this feed into ordered batches of work against the store.
    ///
    /// `incremental_work` is required for [`ScrubFeed::Incremental`]: the caller (the
    /// planner, which owns per-provider planning coverage) computes the remaining work as
    /// a coverage map of ranges — typically the union, across active providers, of each
    /// provider's planning gaps within the store's *synced* coverage. Feeds never claim
    /// time the store hasn't synced.
    pub async fn batches(
        &self,
        store: &dyn Storage,
        incremental_work: Option<&CoverageMap>,
        batch_hint: usize,
    ) -> Result<Vec<FeedBatch>> {
        let batch_hint = batch_hint.max(1);
        match self {
            ScrubFeed::StoreRange { range } => {
                let range = range.clone().unwrap_or(0..u64::MAX);
                let records = store.scrobbles_in_range(range.clone()).await?;
                Ok(chunk_with_claims(records, range, batch_hint))
            }
            ScrubFeed::Incremental { .. } => {
                let work = incremental_work
                    .expect("Incremental feeds require the planner-computed work map");
                let mut batches = Vec::new();
                for segment in work.segments() {
                    let range = segment.range();
                    let records = store.scrobbles_in_range(range.clone()).await?;
                    batches.extend(chunk_with_claims(records, range, batch_hint));
                }
                Ok(batches)
            }
            ScrubFeed::Artist { name, range } => {
                let records = store.artist_scrobbles(name, range.clone()).await?;
                Ok(chunk_spot(records, batch_hint))
            }
            ScrubFeed::Album { artist, album } => {
                let records = store
                    .artist_scrobbles(artist, None)
                    .await?
                    .into_iter()
                    .filter(|r| r.album.as_deref() == Some(album.as_str()))
                    .collect();
                Ok(chunk_spot(records, batch_hint))
            }
            ScrubFeed::Ids(ids) => {
                let mut records = Vec::new();
                for id in ids {
                    match store.get_scrobble(id).await? {
                        Some(record) if !record.deleted => records.push(record),
                        Some(_) => log::debug!("feed: skipping tombstoned {id}"),
                        None => log::warn!("feed: unknown scrobble id {id}"),
                    }
                }
                Ok(chunk_spot(records, batch_hint))
            }
        }
    }
}

/// Chunk coverage-driven records (ascending by uts) into batches whose claims tile the
/// whole `range` — including empty stretches, so known-empty time is claimed too. Chunk
/// boundaries never split a same-second group (a claim boundary at `uts + 1` guarantees
/// every instant is claimed by exactly one batch).
fn chunk_with_claims(
    records: Vec<scrobble_store::ScrobbleRecord>,
    range: std::ops::Range<u64>,
    batch_hint: usize,
) -> Vec<FeedBatch> {
    if records.is_empty() {
        return vec![FeedBatch {
            records: Vec::new(),
            coverage_claim: Some(range),
        }];
    }

    let mut batches: Vec<FeedBatch> = Vec::new();
    let mut cursor = range.start;
    let mut current: Vec<scrobble_store::ScrobbleRecord> = Vec::new();

    for record in records {
        let boundary_needed = current.len() >= batch_hint
            && current.last().is_some_and(|last| last.uts != record.uts);
        if boundary_needed {
            let claim_end = current.last().map(|last| last.uts + 1).unwrap_or(cursor);
            batches.push(FeedBatch {
                records: std::mem::take(&mut current),
                coverage_claim: Some(cursor..claim_end),
            });
            cursor = claim_end;
        }
        current.push(record);
    }
    batches.push(FeedBatch {
        records: current,
        coverage_claim: Some(cursor..range.end),
    });
    batches
}

/// Chunk spot-feed records into batches without coverage claims.
fn chunk_spot(records: Vec<scrobble_store::ScrobbleRecord>, batch_hint: usize) -> Vec<FeedBatch> {
    if records.is_empty() {
        return Vec::new();
    }
    records
        .chunks(batch_hint)
        .map(|chunk| FeedBatch {
            records: chunk.to_vec(),
            coverage_claim: None,
        })
        .collect()
}

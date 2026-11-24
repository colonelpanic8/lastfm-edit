//! Rediscover frequently played tracks whose last recorded play was long ago.

use crate::{CoverageMap, Result, Storage, TrackStats};

pub const YEAR_SECONDS: u64 = 365 * 86_400;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    pub min_plays: u64,
    pub min_gap_days: u64,
    /// 0 ranks by plays, 50 by plays × years away, and 100 by time away.
    pub gap_weight: u8,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            min_plays: 10,
            min_gap_days: 3 * 365,
            gap_weight: 50,
        }
    }
}

pub struct RankedTrack<'a> {
    pub stats: &'a TrackStats,
    pub years_away: f64,
    pub score: f64,
    pub gap_verified: bool,
}

pub struct History {
    tracks: Vec<TrackStats>,
    coverage: CoverageMap,
    pub as_of: u64,
    pub counts_complete: bool,
}

impl History {
    pub async fn load(store: &dyn Storage, now: u64) -> Result<Self> {
        let coverage = store.load_coverage().await?;
        let as_of = coverage
            .last()
            .map_or(now, |s| now.min(s.end.saturating_sub(1)));
        let start = store.load_sync_state().await?.history_start_uts;
        let counts_complete =
            start.is_some_and(|s| s <= as_of && coverage.covers(s..as_of.saturating_add(1)));
        Ok(Self {
            tracks: store.track_stats(None).await?,
            as_of,
            coverage,
            counts_complete,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    pub fn rank(&self, options: Options) -> Vec<RankedTrack<'_>> {
        let min_gap = options.min_gap_days.saturating_mul(86_400);
        let weight = f64::from(options.gap_weight.min(100)) / 100.0;
        let mut rows: Vec<_> = self
            .tracks
            .iter()
            .filter_map(|stats| {
                let gap = self.as_of.checked_sub(stats.last_uts)?;
                if stats.count < options.min_plays || gap < min_gap {
                    return None;
                }
                let years_away = gap as f64 / YEAR_SECONDS as f64;
                let score =
                    (stats.count as f64).powf(2.0 * (1.0 - weight)) * years_away.powf(2.0 * weight);
                Some(RankedTrack {
                    stats,
                    years_away,
                    score,
                    gap_verified: self
                        .coverage
                        .covers(stats.last_uts..self.as_of.saturating_add(1)),
                })
            })
            .collect();
        rows.sort_unstable_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| b.stats.count.cmp(&a.stats.count))
                .then_with(|| a.stats.last_uts.cmp(&b.stats.last_uts))
                .then_with(|| a.stats.artist.cmp(&b.stats.artist))
                .then_with(|| a.stats.track.cmp(&b.stats.track))
        });
        rows
    }
}

//! Synchronization coverage: which time ranges we believe are in agreement with upstream.
//!
//! A [`CoverageMap`] is a set of disjoint, sorted, non-adjacent half-open segments
//! `[start, end)` over Unix timestamps. A covered instant means "the local store and the
//! remote replica agreed about every scrobble at that instant, as of the segment's
//! `verified_at`". The structure is deliberately direction-agnostic — it describes agreement
//! between two replicas, not which one pushed to the other — so the sync direction can be
//! reversed later without changing the model.
//!
//! Invalidation is a first-class operation: [`CoverageMap::subtract`] removes any range from
//! coverage (splitting or shrinking segments as needed), which forces the next sync to
//! re-fetch it.

use serde::{Deserialize, Serialize};
use std::ops::Range;

/// A half-open covered range `[start, end)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Segment {
    /// Inclusive start (Unix seconds).
    pub start: u64,
    /// Exclusive end (Unix seconds).
    pub end: u64,
    /// When agreement over this range was established (Unix seconds). For a segment produced
    /// by merging parts verified at different times, this is the *oldest* constituent
    /// verification — the weakest claim that holds for the whole range.
    pub verified_at: u64,
}

impl Segment {
    pub fn new(start: u64, end: u64, verified_at: u64) -> Self {
        Self {
            start,
            end,
            verified_at,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.start >= self.end
    }

    pub fn range(&self) -> Range<u64> {
        self.start..self.end
    }

    pub fn len(&self) -> u64 {
        self.end.saturating_sub(self.start)
    }

    /// Overlapping *or* touching: half-open `[a,b)` + `[b,c)` fuse into `[a,c)`.
    fn merges_with(&self, other: &Segment) -> bool {
        self.start <= other.end && other.start <= self.end
    }

    /// Strict overlap (shared instants), used by subtraction.
    fn intersects(&self, range: &Range<u64>) -> bool {
        self.start < range.end && range.start < self.end
    }
}

/// How a [`CoverageMap`] mutation changed the map. Emitted so sync progress can be
/// observed and rendered incrementally.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CoverageChange {
    /// A new segment appeared, not touching any existing one.
    Created(Segment),
    /// An existing segment grew (absorbed the inserted range on one or both sides).
    Extended(Segment),
    /// Two or more segments (plus the inserted range) fused into one.
    Merged {
        into: Segment,
        absorbed: Vec<Segment>,
    },
    /// A segment was split in two by an invalidation.
    Split { left: Segment, right: Segment },
    /// A segment lost a piece off one end to an invalidation.
    Shrunk { from: Segment, to: Segment },
    /// A segment was entirely invalidated.
    Removed(Segment),
}

/// The set of time ranges believed synchronized. See module docs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageMap {
    /// Sorted by `start`; pairwise disjoint and non-adjacent.
    segments: Vec<Segment>,
}

impl CoverageMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build from arbitrary segments, normalizing (sorting and fusing) as needed.
    /// Useful when merging coverage files from different machines: the union of two valid
    /// coverage maps over the same upstream is itself valid.
    pub fn from_segments(segments: impl IntoIterator<Item = Segment>) -> Self {
        let mut map = Self::new();
        for seg in segments {
            map.insert(seg);
        }
        map
    }

    /// Mark a range as covered, extending and fusing existing segments as necessary.
    /// Returns what changed (empty if the range was already fully covered and nothing grew).
    pub fn insert(&mut self, seg: Segment) -> Vec<CoverageChange> {
        if seg.is_empty() {
            return Vec::new();
        }

        // Collect every existing segment that overlaps or touches the inserted range.
        let mut absorbed: Vec<Segment> = Vec::new();
        let mut merged = seg;
        self.segments.retain(|existing| {
            if existing.merges_with(&seg) {
                absorbed.push(*existing);
                merged.start = merged.start.min(existing.start);
                merged.end = merged.end.max(existing.end);
                merged.verified_at = merged.verified_at.min(existing.verified_at);
                false
            } else {
                true
            }
        });

        let change = match absorbed.len() {
            0 => Some(CoverageChange::Created(merged)),
            1 if absorbed[0] == merged => None, // fully covered already; nothing changed
            1 => Some(CoverageChange::Extended(merged)),
            _ => Some(CoverageChange::Merged {
                into: merged,
                absorbed,
            }),
        };

        let pos = self
            .segments
            .partition_point(|existing| existing.start < merged.start);
        self.segments.insert(pos, merged);

        change.into_iter().collect()
    }

    /// Invalidate a range: remove it from coverage, splitting or shrinking segments that
    /// straddle its edges. This is the primitive behind every invalidation use case.
    pub fn subtract(&mut self, range: Range<u64>) -> Vec<CoverageChange> {
        if range.start >= range.end {
            return Vec::new();
        }

        let mut changes = Vec::new();
        let mut result: Vec<Segment> = Vec::with_capacity(self.segments.len() + 1);

        for existing in self.segments.drain(..) {
            if !existing.intersects(&range) {
                result.push(existing);
                continue;
            }
            let keeps_left = existing.start < range.start;
            let keeps_right = range.end < existing.end;
            match (keeps_left, keeps_right) {
                (true, true) => {
                    let left = Segment::new(existing.start, range.start, existing.verified_at);
                    let right = Segment::new(range.end, existing.end, existing.verified_at);
                    changes.push(CoverageChange::Split { left, right });
                    result.push(left);
                    result.push(right);
                }
                (true, false) => {
                    let to = Segment::new(existing.start, range.start, existing.verified_at);
                    changes.push(CoverageChange::Shrunk { from: existing, to });
                    result.push(to);
                }
                (false, true) => {
                    let to = Segment::new(range.end, existing.end, existing.verified_at);
                    changes.push(CoverageChange::Shrunk { from: existing, to });
                    result.push(to);
                }
                (false, false) => changes.push(CoverageChange::Removed(existing)),
            }
        }

        self.segments = result;
        changes
    }

    /// Whether a single instant is covered.
    pub fn contains(&self, ts: u64) -> bool {
        let idx = self.segments.partition_point(|seg| seg.start <= ts);
        idx > 0 && ts < self.segments[idx - 1].end
    }

    /// Whether an entire range is covered by a single segment (disjoint segments with a gap
    /// between them do not cover a range spanning the gap).
    pub fn covers(&self, range: Range<u64>) -> bool {
        if range.start >= range.end {
            return true;
        }
        let idx = self
            .segments
            .partition_point(|seg| seg.start <= range.start);
        idx > 0 && range.end <= self.segments[idx - 1].end
    }

    /// The uncovered gaps within `within`, in ascending order.
    pub fn gaps(&self, within: Range<u64>) -> Vec<Range<u64>> {
        let mut gaps = Vec::new();
        if within.start >= within.end {
            return gaps;
        }
        let mut cursor = within.start;
        for seg in &self.segments {
            if seg.end <= cursor {
                continue;
            }
            if seg.start >= within.end {
                break;
            }
            if seg.start > cursor {
                gaps.push(cursor..seg.start.min(within.end));
            }
            cursor = cursor.max(seg.end);
            if cursor >= within.end {
                return gaps;
            }
        }
        if cursor < within.end {
            gaps.push(cursor..within.end);
        }
        gaps
    }

    /// The latest (rightmost) segment, if any. Its `end` is the frontier that
    /// extend-to-present syncs continue from.
    pub fn last(&self) -> Option<&Segment> {
        self.segments.last()
    }

    /// The earliest (leftmost) segment, if any.
    pub fn first(&self) -> Option<&Segment> {
        self.segments.first()
    }

    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// Total covered duration in seconds.
    pub fn total_covered(&self) -> u64 {
        self.segments.iter().map(Segment::len).sum()
    }
}

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod tests;

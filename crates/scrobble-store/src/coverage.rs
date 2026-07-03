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
mod tests {
    use super::*;

    fn seg(start: u64, end: u64) -> Segment {
        Segment::new(start, end, 1_000)
    }

    #[test]
    fn insert_into_empty_creates() {
        let mut map = CoverageMap::new();
        let changes = map.insert(seg(10, 20));
        assert_eq!(changes, vec![CoverageChange::Created(seg(10, 20))]);
        assert!(map.contains(10));
        assert!(map.contains(19));
        assert!(!map.contains(20)); // half-open
        assert!(!map.contains(9));
    }

    #[test]
    fn empty_segment_is_noop() {
        let mut map = CoverageMap::new();
        assert!(map.insert(seg(10, 10)).is_empty());
        assert!(map.is_empty());
    }

    #[test]
    fn adjacent_segments_fuse() {
        let mut map = CoverageMap::new();
        map.insert(seg(10, 20));
        let changes = map.insert(seg(20, 30));
        assert!(matches!(changes[0], CoverageChange::Extended(s) if s.range() == (10..30)));
        assert_eq!(map.segments().len(), 1);
    }

    #[test]
    fn gap_filling_merges_two_segments() {
        let mut map = CoverageMap::new();
        map.insert(seg(10, 20));
        map.insert(seg(30, 40));
        assert_eq!(map.segments().len(), 2);
        let changes = map.insert(seg(20, 30));
        match &changes[0] {
            CoverageChange::Merged { into, absorbed } => {
                assert_eq!(into.range(), 10..40);
                assert_eq!(absorbed.len(), 2);
            }
            other => panic!("expected Merged, got {other:?}"),
        }
        assert_eq!(map.segments().len(), 1);
    }

    #[test]
    fn reinsert_of_covered_range_reports_nothing() {
        let mut map = CoverageMap::new();
        map.insert(seg(10, 40));
        assert!(map.insert(seg(15, 25)).is_empty());
        assert_eq!(map.segments().len(), 1);
        assert_eq!(map.segments()[0].range(), 10..40);
    }

    #[test]
    fn merged_verified_at_is_conservative_min() {
        let mut map = CoverageMap::new();
        map.insert(Segment::new(10, 20, 100));
        map.insert(Segment::new(20, 30, 500));
        assert_eq!(map.segments()[0].verified_at, 100);
    }

    #[test]
    fn subtract_splits_a_straddled_segment() {
        let mut map = CoverageMap::new();
        map.insert(seg(10, 40));
        let changes = map.subtract(20..30);
        match &changes[0] {
            CoverageChange::Split { left, right } => {
                assert_eq!(left.range(), 10..20);
                assert_eq!(right.range(), 30..40);
            }
            other => panic!("expected Split, got {other:?}"),
        }
        assert!(map.contains(15));
        assert!(!map.contains(25));
        assert!(map.contains(35));
    }

    #[test]
    fn subtract_shrinks_edges_and_removes_inner() {
        let mut map = CoverageMap::new();
        map.insert(seg(10, 20));
        map.insert(seg(30, 40));
        map.insert(seg(50, 60));
        // Range clipping the right of the first, all of the second, left of the third.
        let changes = map.subtract(15..55);
        assert_eq!(changes.len(), 3);
        assert!(matches!(&changes[0], CoverageChange::Shrunk { to, .. } if to.range() == (10..15)));
        assert!(matches!(&changes[1], CoverageChange::Removed(s) if s.range() == (30..40)));
        assert!(matches!(&changes[2], CoverageChange::Shrunk { to, .. } if to.range() == (55..60)));
        assert_eq!(map.gaps(10..60), vec![15..55]);
    }

    #[test]
    fn subtract_touching_boundary_changes_nothing() {
        let mut map = CoverageMap::new();
        map.insert(seg(10, 20));
        // Half-open: subtracting [20, 30) shares no instants with [10, 20).
        assert!(map.subtract(20..30).is_empty());
        assert_eq!(map.segments()[0].range(), 10..20);
    }

    #[test]
    fn gaps_enumeration() {
        let mut map = CoverageMap::new();
        map.insert(seg(10, 20));
        map.insert(seg(30, 40));
        assert_eq!(map.gaps(0..50), vec![0..10, 20..30, 40..50]);
        assert_eq!(map.gaps(10..40), vec![20..30]);
        assert_eq!(map.gaps(12..18), Vec::<Range<u64>>::new());
        assert_eq!(map.gaps(15..35), vec![20..30]);
        // Degenerate window.
        assert_eq!(map.gaps(5..5), Vec::<Range<u64>>::new());
    }

    #[test]
    fn covers_requires_single_segment() {
        let mut map = CoverageMap::new();
        map.insert(seg(10, 20));
        map.insert(seg(30, 40));
        assert!(map.covers(10..20));
        assert!(map.covers(12..18));
        assert!(!map.covers(10..40)); // spans a gap
        assert!(map.covers(15..15)); // empty range trivially covered
    }

    #[test]
    fn union_via_from_segments_is_valid() {
        // Two machines' coverage maps merged by union: normalization fuses them.
        let a = [seg(10, 30), seg(50, 60)];
        let b = [seg(20, 55)];
        let merged = CoverageMap::from_segments(a.into_iter().chain(b));
        assert_eq!(merged.segments().len(), 1);
        assert_eq!(merged.segments()[0].range(), 10..60);
    }

    /// Deterministic xorshift so the model test needs no external RNG crate.
    struct XorShift(u64);
    impl XorShift {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
    }

    #[test]
    fn randomized_against_naive_seconds_model() {
        const UNIVERSE: u64 = 200;
        let mut rng = XorShift(0x5EED_CAFE_F00D_D00D);
        let mut map = CoverageMap::new();
        let mut model = vec![false; UNIVERSE as usize];

        for step in 0..2_000 {
            let a = rng.next() % UNIVERSE;
            let b = rng.next() % UNIVERSE;
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            if rng.next() % 2 == 0 {
                map.insert(Segment::new(lo, hi, step));
                model[lo as usize..hi as usize].fill(true);
            } else {
                map.subtract(lo..hi);
                model[lo as usize..hi as usize].fill(false);
            }

            // Point-wise agreement with the naive model.
            for ts in 0..UNIVERSE {
                assert_eq!(
                    map.contains(ts),
                    model[ts as usize],
                    "step {step}: mismatch at ts {ts}"
                );
            }

            // Structural invariants: sorted, disjoint, non-adjacent, non-empty.
            for pair in map.segments().windows(2) {
                assert!(pair[0].end < pair[1].start, "step {step}: {pair:?}");
            }
            for seg in map.segments() {
                assert!(!seg.is_empty(), "step {step}: empty segment {seg:?}");
            }

            // gaps() complements contains() over the whole universe.
            let gaps = map.gaps(0..UNIVERSE);
            let mut in_gap = vec![false; UNIVERSE as usize];
            for gap in &gaps {
                in_gap[gap.start as usize..gap.end as usize].fill(true);
            }
            for ts in 0..UNIVERSE {
                assert_eq!(
                    in_gap[ts as usize],
                    !map.contains(ts),
                    "step {step}, ts {ts}"
                );
            }
        }
    }
}

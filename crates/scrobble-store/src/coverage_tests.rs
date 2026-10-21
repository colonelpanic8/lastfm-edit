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
        if rng.next().is_multiple_of(2) {
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

//! Pure canonical-album policy. The only place it is encoded.

use super::mb::{
    ArtistCredit, PartialDate, PrimaryType, ReleaseGroup, ReleaseGroupId, ReleaseRef, SecondaryType,
};
use super::title::fold;
use std::cmp::Reverse;
use std::collections::BTreeMap;

/// Preference ladder for the album a recording "belongs" to. Only `PlainAlbum` targets
/// can be auto-applied.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum Rung {
    PlainAlbum,
    /// A plain album where a missing date leaves open whether its first release carried
    /// the recording, rather than only a later reissue.
    UndatedAlbum,
    SoundtrackAlbum,
    PlainEp,
    Single,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Canonical {
    pub group: ReleaseGroup,
    pub rung: Rung,
    /// A Single release group of this recording predates the chosen album by more than
    /// two years. Forces review.
    pub single_first: bool,
}

/// Choose the canonical release group over the official releases of every recording in a
/// master. Candidates must carry the same artist-id set as `credit` (excludes Various
/// Artists compilations and other artists' albums with guest spots), and must have carried
/// the recording on their first release: a deluxe reissue's bonus track doesn't belong to
/// the original album. Earliest `first_release` within the best rung wins; undated sorts
/// last; ties break on member release count (more first), then id.
pub(crate) fn choose_canonical(
    releases: &[ReleaseRef],
    credit: &ArtistCredit,
    recording_title: &str,
) -> Option<Canonical> {
    let artists = credit.artist_ids();
    let mut groups: BTreeMap<&ReleaseGroupId, Membership> = BTreeMap::new();
    for release in releases {
        if release.official && release.group.credit.artist_ids() == artists {
            groups
                .entry(&release.group.id)
                .or_insert_with(|| Membership {
                    group: &release.group,
                    members: 0,
                    earliest: None,
                    undated: false,
                })
                .add(release);
        }
    }
    let eligible: Vec<(&ReleaseGroup, Appearance, usize)> = groups
        .values()
        .filter_map(|m| match m.appearance() {
            Appearance::Later => None,
            appearance => Some((m.group, appearance, m.members)),
        })
        .collect();

    let (group, rung) = eligible
        .iter()
        .filter_map(|&(group, appearance, members)| {
            let rung = match rung(group, recording_title)? {
                Rung::PlainAlbum if appearance == Appearance::Undated => Rung::UndatedAlbum,
                rung => rung,
            };
            Some((group, rung, members))
        })
        .min_by_key(|&(group, rung, members)| {
            (
                rung,
                group.first_release.is_none(),
                group.first_release,
                Reverse(members),
                &group.id,
            )
        })
        .map(|(group, rung, _)| (group, rung))?;

    let single_first = rung != Rung::Single
        && group.first_release.is_some_and(|album| {
            eligible.iter().any(|(other, ..)| {
                is_plain(other, PrimaryType::Single)
                    && other
                        .first_release
                        .is_some_and(|single| album.year.saturating_sub(single.year) > 2)
            })
        });

    Some(Canonical {
        group: group.clone(),
        rung,
        single_first,
    })
}

/// The master's releases inside one release group.
struct Membership<'a> {
    group: &'a ReleaseGroup,
    members: usize,
    earliest: Option<PartialDate>,
    undated: bool,
}

/// Whether a group's first release carried the recording.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Appearance {
    /// A member release is dated within a year of the group's first release.
    FirstRelease,
    /// A missing date leaves it open.
    Undated,
    /// Only releases dated over a year later carry it (reissues with bonus tracks).
    Later,
}

impl Membership<'_> {
    fn add(&mut self, release: &ReleaseRef) {
        self.members += 1;
        match release.date {
            Some(date) => self.earliest = Some(self.earliest.map_or(date, |e| e.min(date))),
            None => self.undated = true,
        }
    }

    fn appearance(&self) -> Appearance {
        match (self.group.first_release, self.earliest) {
            (Some(first), Some(earliest)) if earliest.year.saturating_sub(first.year) <= 1 => {
                Appearance::FirstRelease
            }
            (Some(_), Some(_)) if !self.undated => Appearance::Later,
            _ => Appearance::Undated,
        }
    }
}

fn is_plain(group: &ReleaseGroup, primary: PrimaryType) -> bool {
    group.primary == Some(primary) && group.secondary.is_empty()
}

fn rung(group: &ReleaseGroup, recording_title: &str) -> Option<Rung> {
    match (group.primary?, group.secondary.as_slice()) {
        (PrimaryType::Album, []) => Some(Rung::PlainAlbum),
        (PrimaryType::Album, [SecondaryType::Soundtrack]) => Some(Rung::SoundtrackAlbum),
        (PrimaryType::Ep, []) => Some(Rung::PlainEp),
        (PrimaryType::Single, []) if fold(&group.title) == fold(recording_title) => {
            Some(Rung::Single)
        }
        _ => None,
    }
}

/// Why a scrobble on release group `scrobbled` may move to `canonical`. The caller has
/// already proven recording identity (same master).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AlbumMove {
    /// Same release group, different album string (edition/deluxe/remaster title).
    EditionToGroup,
    FromCompilation,
    /// Single/EP -> album. Review only.
    FromSingleOrEp,
}

/// `None` means the scrobbled album stays: plain albums, and anything Live, Remix, DJ-mix,
/// Demo, Soundtrack, or Spokenword (its type says the performance belongs there).
pub(crate) fn album_move_allowed(
    scrobbled: &ReleaseGroup,
    canonical: &Canonical,
) -> Option<AlbumMove> {
    if scrobbled.id == canonical.group.id {
        return Some(AlbumMove::EditionToGroup);
    }
    match (scrobbled.primary, scrobbled.secondary.as_slice()) {
        (_, [SecondaryType::Compilation]) => Some(AlbumMove::FromCompilation),
        (Some(PrimaryType::Single), []) if canonical.rung < Rung::Single => {
            Some(AlbumMove::FromSingleOrEp)
        }
        (Some(PrimaryType::Ep), []) if canonical.rung < Rung::PlainEp => {
            Some(AlbumMove::FromSingleOrEp)
        }
        _ => None,
    }
}

/// MusicBrainz fixtures shared by policy tests.
#[cfg(test)]
#[path = "canonical_examples_tests.rs"]
pub(crate) mod examples;

#[cfg(test)]
#[path = "canonical_tests.rs"]
mod tests;

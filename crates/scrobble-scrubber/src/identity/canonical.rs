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
pub(crate) mod examples {
    use super::super::mb::{
        ArtistCredit, ArtistId, CreditPart, PartialDate, PrimaryType, ReleaseGroup, ReleaseGroupId,
        ReleaseRef, SecondaryType,
    };

    pub const CLAPTON: &str = "618b6900-0618-4f1e-b835-bccb17f84294";
    pub const VARIOUS: &str = "89ad4ac3-39f7-470e-963a-56509c546377";

    pub fn credit(parts: &[(&str, &str, &str)]) -> ArtistCredit {
        ArtistCredit {
            parts: parts
                .iter()
                .map(|&(id, name, joinphrase)| CreditPart {
                    artist: ArtistId(id.to_string()),
                    name: name.to_string(),
                    joinphrase: joinphrase.to_string(),
                })
                .collect(),
        }
    }

    pub fn clapton() -> ArtistCredit {
        credit(&[(CLAPTON, "Eric Clapton", "")])
    }

    pub fn group(
        id: &str,
        title: &str,
        primary: PrimaryType,
        secondary: &[SecondaryType],
        first_release: &str,
        credit: ArtistCredit,
    ) -> ReleaseGroup {
        ReleaseGroup {
            id: ReleaseGroupId(id.to_string()),
            title: title.to_string(),
            primary: Some(primary),
            secondary: secondary.to_vec(),
            first_release: PartialDate::parse(first_release),
            credit,
        }
    }

    pub fn release(title: &str, group: &ReleaseGroup) -> ReleaseRef {
        ReleaseRef {
            title: title.to_string(),
            official: true,
            date: group.first_release,
            group: group.clone(),
        }
    }

    pub fn slowhand() -> ReleaseGroup {
        group(
            "02c2b0c7-065d-38b4-8ac0-3391839f2418",
            "Slowhand",
            PrimaryType::Album,
            &[],
            "1977-11-25",
            clapton(),
        )
    }

    pub fn cream_of_clapton() -> ReleaseGroup {
        group(
            "976d6745-0091-39f1-ae60-c142986485a5",
            "The Cream of Clapton",
            PrimaryType::Album,
            &[SecondaryType::Compilation],
            "1995-03-13",
            clapton(),
        )
    }

    pub fn incredible_jazz_guitar() -> ReleaseGroup {
        group(
            "incredible-jazz-guitar",
            "The Incredible Jazz Guitar of Wes Montgomery",
            PrimaryType::Album,
            &[],
            "1960",
            credit(&[("wes-montgomery", "Wes Montgomery", "")]),
        )
    }

    pub fn smokin_at_the_half_note() -> ReleaseGroup {
        group(
            "smokin-at-the-half-note",
            "Smokin' at the Half Note",
            PrimaryType::Album,
            &[SecondaryType::Live],
            "1965",
            credit(&[
                ("wes-montgomery", "Wes Montgomery", " & "),
                ("wynton-kelly-trio", "Wynton Kelly Trio", ""),
            ]),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::examples::*;
    use super::*;

    fn cocaine_master_releases() -> Vec<ReleaseRef> {
        let slowhand = slowhand();
        let mut bootleg = release("Slowhand Live in Tokyo", &slowhand);
        bootleg.official = false;
        let undated_slowhand = ReleaseRef {
            date: None,
            ..release("Slowhand", &slowhand)
        };
        vec![
            release("The Cream of Clapton", &cream_of_clapton()),
            release(
                "Timepieces: The Best of Eric Clapton",
                &group(
                    "timepieces",
                    "Time Pieces: The Best of Eric Clapton",
                    PrimaryType::Album,
                    &[SecondaryType::Compilation],
                    "1982",
                    clapton(),
                ),
            ),
            release(
                "Soft Metal Ballads",
                &group(
                    "soft-metal-ballads",
                    "Soft Metal Ballads",
                    PrimaryType::Album,
                    &[SecondaryType::Compilation],
                    "1991",
                    credit(&[(VARIOUS, "Various Artists", "")]),
                ),
            ),
            release(
                "Lay Down Sally",
                &group(
                    "lay-down-sally",
                    "Lay Down Sally",
                    PrimaryType::Single,
                    &[],
                    "1977-11",
                    clapton(),
                ),
            ),
            release(
                "Just One Night",
                &group(
                    "just-one-night",
                    "Just One Night",
                    PrimaryType::Album,
                    &[SecondaryType::Live],
                    "1980-04",
                    clapton(),
                ),
            ),
            release("Slowhand", &slowhand),
            release("Slowhand 35th Anniversary", &slowhand),
            undated_slowhand,
            bootleg,
        ]
    }

    #[test]
    fn cocaine_belongs_to_slowhand() {
        assert_eq!(
            choose_canonical(&cocaine_master_releases(), &clapton(), "Cocaine"),
            Some(Canonical {
                group: slowhand(),
                rung: Rung::PlainAlbum,
                single_first: false,
            })
        );
    }

    #[test]
    fn ladder_prefers_album_then_soundtrack_then_ep_then_same_titled_single() {
        let artist = || credit(&[("courtney-barnett", "Courtney Barnett", "")]);
        let single = group(
            "history-eraser-single",
            "History Eraser",
            PrimaryType::Single,
            &[],
            "2013",
            artist(),
        );
        let other_single = group(
            "avant-gardener",
            "Avant Gardener",
            PrimaryType::Single,
            &[],
            "2012",
            artist(),
        );
        let ep = group(
            "a-sea-of-split-peas",
            "The Double EP: A Sea of Split Peas",
            PrimaryType::Ep,
            &[],
            "2013-10",
            artist(),
        );
        let soundtrack = group(
            "film-ost",
            "Some Film (Original Motion Picture Soundtrack)",
            PrimaryType::Album,
            &[SecondaryType::Soundtrack],
            "2014",
            artist(),
        );
        let album = group(
            "sometimes-i-sit",
            "Sometimes I Sit and Think, and Sometimes I Just Sit",
            PrimaryType::Album,
            &[],
            "2015-03",
            artist(),
        );

        let pick = |groups: &[&ReleaseGroup]| {
            let releases: Vec<ReleaseRef> = groups.iter().map(|g| release(&g.title, g)).collect();
            choose_canonical(&releases, &artist(), "History Eraser").map(|c| (c.group.id.0, c.rung))
        };
        assert_eq!(
            pick(&[&single, &ep, &soundtrack, &album]),
            Some(("sometimes-i-sit".into(), Rung::PlainAlbum))
        );
        assert_eq!(
            pick(&[&single, &ep, &soundtrack]),
            Some(("film-ost".into(), Rung::SoundtrackAlbum))
        );
        assert_eq!(
            pick(&[&single, &ep]),
            Some(("a-sea-of-split-peas".into(), Rung::PlainEp))
        );
        assert_eq!(
            pick(&[&single, &other_single]),
            Some(("history-eraser-single".into(), Rung::Single))
        );
        assert_eq!(pick(&[&other_single]), None);
    }

    #[test]
    fn earliest_dated_group_wins_within_a_rung() {
        let stones = || credit(&[("rolling-stones", "The Rolling Stones", "")]);
        let sticky = group(
            "4a4213d0-f224-3d51-bcca-476f61162681",
            "Sticky Fingers",
            PrimaryType::Album,
            &[],
            "1971-04-23",
            stones(),
        );
        let reissue_titled = group(
            "sticky-fingers-live-at-fonda",
            "Sticky Fingers Live at the Fonda Theatre",
            PrimaryType::Album,
            &[],
            "2017-09-29",
            stones(),
        );
        let undated = ReleaseGroup {
            first_release: None,
            ..group(
                "undated",
                "Undated Album",
                PrimaryType::Album,
                &[],
                "",
                stones(),
            )
        };
        let releases = vec![
            release("Undated Album", &undated),
            release("Sticky Fingers Live at the Fonda Theatre", &reissue_titled),
            release(
                "Hot Rocks 1964–1971",
                &group(
                    "6e33402c-62db-3dc9-af86-2c1bef99ce2f",
                    "Hot Rocks 1964–1971",
                    PrimaryType::Album,
                    &[SecondaryType::Compilation],
                    "1971-12-20",
                    stones(),
                ),
            ),
            release("Sticky Fingers", &sticky),
        ];
        let canonical = choose_canonical(&releases, &stones(), "Wild Horses").unwrap();
        assert_eq!(canonical.group.title, "Sticky Fingers");
    }

    #[test]
    fn credit_must_match_the_recording_artists_exactly() {
        let dominos = group(
            "layla",
            "Layla and Other Assorted Love Songs",
            PrimaryType::Album,
            &[],
            "1970-11",
            credit(&[("derek-and-the-dominos", "Derek and the Dominos", "")]),
        );
        let duet = group(
            "riding-with-the-king",
            "Riding With the King",
            PrimaryType::Album,
            &[],
            "2000-06",
            credit(&[
                ("bb-king", "B.B. King", " & "),
                (CLAPTON, "Eric Clapton", ""),
            ]),
        );
        let releases = vec![
            release(&dominos.title, &dominos),
            release(&duet.title, &duet),
        ];
        assert_eq!(
            choose_canonical(&releases, &clapton(), "Key to the Highway"),
            None
        );
    }

    #[test]
    fn a_track_only_on_a_later_reissue_does_not_belong_to_the_album() {
        let radiohead = || credit(&[("a74b1b7f-71a5-4011-9441-d0b5e4122711", "Radiohead", "")]);
        let the_bends = group(
            "the-bends",
            "The Bends",
            PrimaryType::Album,
            &[],
            "1995-03-13",
            radiohead(),
        );
        let best_of = group(
            "the-best-of",
            "The Best Of",
            PrimaryType::Album,
            &[SecondaryType::Compilation],
            "2008-06-02",
            radiohead(),
        );
        let street_spirit = group(
            "street-spirit",
            "Street Spirit (Fade Out)",
            PrimaryType::Single,
            &[],
            "1996-01-22",
            radiohead(),
        );
        let reissue = |date: &str| ReleaseRef {
            date: PartialDate::parse(date),
            ..release("The Bends (Collector's Edition)", &the_bends)
        };
        let pick = |bends: Vec<ReleaseRef>, title: &str| {
            let mut releases = vec![
                release("The Best Of", &best_of),
                release(&street_spirit.title, &street_spirit),
            ];
            releases.extend(bends);
            choose_canonical(&releases, &radiohead(), title).map(|c| (c.group.id.0, c.rung))
        };

        assert_eq!(pick(vec![reissue("2009-03-24")], "Talk Show Host"), None);
        assert_eq!(
            pick(vec![reissue("")], "Talk Show Host"),
            Some(("the-bends".into(), Rung::UndatedAlbum))
        );
        assert_eq!(
            pick(
                vec![release("The Bends", &the_bends), reissue("2009-03-24")],
                "Just"
            ),
            Some(("the-bends".into(), Rung::PlainAlbum))
        );
        assert_eq!(
            pick(vec![reissue("1996-02"), reissue("2009-03-24")], "Just"),
            Some(("the-bends".into(), Rung::PlainAlbum))
        );
    }

    #[test]
    fn single_years_before_the_album_is_flagged() {
        let holly = || credit(&[("buddy-holly", "Buddy Holly", "")]);
        let single = group(
            "peggy-sue-got-married",
            "Peggy Sue Got Married",
            PrimaryType::Single,
            &[],
            "1959-07",
            holly(),
        );
        let album = group(
            "reminiscing",
            "Reminiscing",
            PrimaryType::Album,
            &[],
            "1963-02",
            holly(),
        );
        let releases = vec![
            release(&single.title, &single),
            release(&album.title, &album),
        ];
        let canonical = choose_canonical(&releases, &holly(), "Peggy Sue Got Married").unwrap();
        assert_eq!(
            (canonical.group.id.0.as_str(), canonical.single_first),
            ("reminiscing", true)
        );
    }

    #[test]
    fn four_on_six_live_recording_has_no_canonical_and_never_moves() {
        let live = smokin_at_the_half_note();
        let wes = credit(&[
            ("wes-montgomery", "Wes Montgomery", " & "),
            ("wynton-kelly-trio", "Wynton Kelly Trio", ""),
        ]);
        assert_eq!(
            choose_canonical(&[release(&live.title, &live)], &wes, "Four on Six"),
            None
        );

        let studio = Canonical {
            group: incredible_jazz_guitar(),
            rung: Rung::PlainAlbum,
            single_first: false,
        };
        assert_eq!(album_move_allowed(&live, &studio), None);
    }

    #[test]
    fn album_moves_by_scrobbled_group_type() {
        let canonical = Canonical {
            group: slowhand(),
            rung: Rung::PlainAlbum,
            single_first: false,
        };
        let single_canonical = Canonical {
            rung: Rung::Single,
            ..canonical.clone()
        };
        let ep_canonical = Canonical {
            rung: Rung::PlainEp,
            ..canonical.clone()
        };
        let other = |primary, secondary: &[SecondaryType]| {
            group("other", "Other", primary, secondary, "1980", clapton())
        };
        use PrimaryType::{Album, Ep, Single};
        use SecondaryType::{Compilation, DjMix, Live, Remix, Soundtrack, Spokenword};

        let cases = [
            (slowhand(), &canonical, Some(AlbumMove::EditionToGroup)),
            (
                cream_of_clapton(),
                &canonical,
                Some(AlbumMove::FromCompilation),
            ),
            (
                other(Single, &[]),
                &canonical,
                Some(AlbumMove::FromSingleOrEp),
            ),
            (other(Ep, &[]), &canonical, Some(AlbumMove::FromSingleOrEp)),
            (
                other(Single, &[]),
                &ep_canonical,
                Some(AlbumMove::FromSingleOrEp),
            ),
            (other(Ep, &[]), &ep_canonical, None),
            (other(Single, &[]), &single_canonical, None),
            (other(Album, &[]), &canonical, None),
            (other(Album, &[Live]), &canonical, None),
            (other(Album, &[Compilation, Live]), &canonical, None),
            (other(Album, &[Remix]), &canonical, None),
            (other(Album, &[DjMix]), &canonical, None),
            (other(Album, &[Soundtrack]), &canonical, None),
            (other(Album, &[Spokenword]), &canonical, None),
        ];
        for (scrobbled, canonical, expected) in cases {
            assert_eq!(
                album_move_allowed(&scrobbled, canonical),
                expected,
                "{}",
                scrobbled.describe()
            );
        }
    }
}

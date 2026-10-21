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

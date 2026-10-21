use super::super::canonical::examples::*;
use super::super::mb::{ArtistCredit, PrimaryType, SecondaryType};
use super::super::naming::NamingJudge;
use super::*;

const ISRC: &str = "NLF057790024";

fn subject(artist: &str, track: &str, album: &str, album_artist: Option<&str>) -> Subject {
    Subject {
        artist: artist.to_string(),
        track: track.to_string(),
        album: Some(album.to_string()),
        album_artist: album_artist.map(str::to_string),
    }
}

fn outcome(
    title: &str,
    scrobbled: ReleaseGroup,
    canonical: Option<Canonical>,
    proof: Proof,
) -> Outcome {
    Outcome {
        master: vec![
            RecordingId("aca688be-742b-4c44-96b9-2278bdc483c3".to_string()),
            RecordingId("40ae4c06-3799-413b-b879-101dec39ac44".to_string()),
        ],
        title: title.to_string(),
        scrobbled_group: Some(scrobbled),
        canonical,
        proof,
    }
}

fn plain_album(group: ReleaseGroup) -> Option<Canonical> {
    Some(Canonical {
        group,
        rung: Rung::PlainAlbum,
        single_first: false,
    })
}

fn shared_isrc(len_delta_ms: u32) -> Proof {
    Proof::SharedIsrc {
        isrc: ISRC.to_string(),
        len_delta_ms,
    }
}

fn target(title: &str, album: &str, album_artist: Option<&str>) -> Target {
    Target {
        title: title.to_string(),
        album: Some(album.to_string()),
        album_artist: album_artist.map(str::to_string),
    }
}

fn cocaine_on_cream() -> (Subject, Outcome) {
    (
        subject(
            "Eric Clapton",
            "Cocaine",
            "The Cream of Clapton",
            Some("Various Artists"),
        ),
        outcome(
            "Cocaine",
            cream_of_clapton(),
            plain_album(slowhand()),
            shared_isrc(2_107),
        ),
    )
}

#[test]
fn cocaine_moves_off_the_compilation_to_slowhand_automatically() {
    let (subject, outcome) = cocaine_on_cream();
    let proposal = derive_edit(
        &subject,
        &outcome,
        Chosen::Deterministic,
        &[],
        &MovePolicy::default(),
        Naming::reviewer(),
    )
    .proposal
    .unwrap();
    assert_eq!(
        proposal.edit,
        ScrobbleEdit {
            track_name_original: Some("Cocaine".to_string()),
            album_name_original: Some("The Cream of Clapton".to_string()),
            artist_name_original: "Eric Clapton".to_string(),
            album_artist_name_original: Some("Various Artists".to_string()),
            track_name: Some("Cocaine".to_string()),
            album_name: Some("Slowhand".to_string()),
            artist_name: "Eric Clapton".to_string(),
            album_artist_name: Some("Eric Clapton".to_string()),
            timestamp: None,
            edit_all: true,
        }
    );
    assert_eq!(
        proposal.target,
        target("Cocaine", "Slowhand", Some("Eric Clapton"))
    );
    assert!(!proposal.approval.requires_confirmation());
    assert_eq!(
            proposal.rationale,
            "Album: The Cream of Clapton (Album + Compilation, 1995) -> Slowhand (Album, 1977) (off a compilation)\n\
             Proof: recordings share ISRC NLF057790024, lengths 2.1 s apart"
        );
}

#[test]
fn edition_fixes_change_only_the_edition_wording() {
    let alabama_shakes = || credit(&[("alabama-shakes", "Alabama Shakes", "")]);
    let sound_and_color = ReleaseGroup {
        credit: ArtistCredit::default(),
        ..group(
            "sound-and-color",
            "Sound & Color",
            PrimaryType::Album,
            &[],
            "2015-04-21",
            alabama_shakes(),
        )
    };
    let whos_next = group(
        "whos-next",
        "Who\u{2019}s Next",
        PrimaryType::Album,
        &[],
        "1971-08-14",
        credit(&[("the-who", "The Who", "")]),
    );
    let cases = [
        (
            subject(
                "Alabama Shakes",
                "Gimme All Your Love",
                "Sound & Color (Deluxe Edition)",
                Some("Alabama Shakes"),
            ),
            "Gimme All Your Love",
            sound_and_color,
            target(
                "Gimme All Your Love",
                "Sound & Color",
                Some("Alabama Shakes"),
            ),
        ),
        (
            subject(
                "The Who",
                "Won't Get Fooled Again - Remastered",
                "Who's Next (Deluxe Edition)",
                None,
            ),
            "Won\u{2019}t Get Fooled Again",
            whos_next,
            target("Won't Get Fooled Again", "Who's Next", None),
        ),
    ];
    for (subject, title, group, expected) in cases {
        let outcome = Outcome {
            master: vec![],
            title: title.to_string(),
            scrobbled_group: Some(group.clone()),
            canonical: plain_album(group),
            proof: Proof::SameRecording,
        };
        let proposal = derive_edit(
            &subject,
            &outcome,
            Chosen::Deterministic,
            &[],
            &MovePolicy::default(),
            Naming::reviewer(),
        )
        .proposal
        .unwrap();
        assert_eq!(
            (proposal.target, proposal.approval.requires_confirmation()),
            (expected, false),
            "{}",
            subject.track
        );
    }
}

#[test]
fn edition_retitles_follow_the_albums_naming_verdict() {
    let group = group(
        "8be83468-4917-390d-82b6-d890f058cd22",
        "John Lennon/Plastic Ono Band",
        PrimaryType::Album,
        &[],
        "1970-12-11",
        credit(&[("john-lennon", "John Lennon", "")]),
    );
    let question = NamingQuestion::album(
        "John Lennon",
        "Plastic Ono Band",
        "John Lennon/Plastic Ono Band",
    );
    let judged = |verdict| {
        HashMap::from([(
            question.clone(),
            NamingJudgement {
                verdict,
                reason: "Restores the official title.".into(),
                by: NamingJudge::Llm {
                    model: "claude:opus".into(),
                    prompt_version: 1,
                },
                at: 1,
            },
        )])
    };
    let no_verdicts = HashMap::new();
    let musicbrainz = judged(NamingVerdict::MusicBrainz);
    let keep = judged(NamingVerdict::Keep);
    let unsure = judged(NamingVerdict::Unsure);
    let retitled = |track: &str| target(track, "John Lennon/Plastic Ono Band", None);

    let cases = [
        (
            "musicbrainz: retitled automatically",
            "Mother",
            "Plastic Ono Band",
            &musicbrainz,
            RetitleJudge::Llm,
            Some((retitled("Mother"), false)),
        ),
        (
            "keep: no album change",
            "Mother",
            "Plastic Ono Band",
            &keep,
            RetitleJudge::Llm,
            None,
        ),
        (
            "keep: the edition suffix still goes",
            "Mother",
            "Plastic Ono Band (Remastered)",
            &keep,
            RetitleJudge::Llm,
            Some((target("Mother", "Plastic Ono Band", None), false)),
        ),
        (
            "keep: the rest of the edit stands",
            "Mother - Remastered 2010",
            "Plastic Ono Band",
            &keep,
            RetitleJudge::Llm,
            Some((target("Mother", "Plastic Ono Band", None), false)),
        ),
        (
            "unsure: for review",
            "Mother",
            "Plastic Ono Band",
            &unsure,
            RetitleJudge::Llm,
            Some((retitled("Mother"), true)),
        ),
        (
            "no verdict and no LLM: for review",
            "Mother",
            "Plastic Ono Band",
            &no_verdicts,
            RetitleJudge::Reviewer,
            Some((retitled("Mother"), true)),
        ),
        (
            "no verdict yet: the album waits for the LLM",
            "Mother",
            "Plastic Ono Band",
            &no_verdicts,
            RetitleJudge::Llm,
            None,
        ),
        (
            "no verdict yet: the title fix goes ahead",
            "Mother - Remastered 2010",
            "Plastic Ono Band",
            &no_verdicts,
            RetitleJudge::Llm,
            Some((target("Mother", "Plastic Ono Band", None), false)),
        ),
    ];
    for (name, track, album, verdicts, judge, expected) in cases {
        let outcome = outcome(
            "Mother",
            group.clone(),
            plain_album(group.clone()),
            Proof::SameRecording,
        );
        let actual = derive_edit(
            &subject("John Lennon", track, album, None),
            &outcome,
            Chosen::Deterministic,
            &[],
            &MovePolicy::default(),
            Naming { verdicts, judge },
        )
        .proposal
        .map(|p| (p.target, p.approval.requires_confirmation()));
        assert_eq!(actual, expected, "{name}");
    }

    let proposal = derive_edit(
        &subject("John Lennon", "Mother", "Plastic Ono Band", None),
        &outcome(
            "Mother",
            group.clone(),
            plain_album(group),
            Proof::SameRecording,
        ),
        Chosen::Deterministic,
        &[],
        &MovePolicy::default(),
        Naming {
            verdicts: &musicbrainz,
            judge: RetitleJudge::Llm,
        },
    )
    .proposal
    .unwrap();
    assert_eq!(
            proposal.rationale,
            "Album: John Lennon/Plastic Ono Band (Album, 1970) -> John Lennon/Plastic Ono Band (Album, 1970) (edition title of the same release group)\n\
             Naming: use MusicBrainz's title (claude:opus): Restores the official title.\n\
             Proof: the same MusicBrainz recording"
        );
}

/// Picked among options: the release group `album` names, and the recording's title.
fn picked(track: &str, album: &str, live: bool, year: &str) -> Outcome {
    let secondary: &[SecondaryType] = if live { &[SecondaryType::Live] } else { &[] };
    let group = group(
        album,
        album,
        PrimaryType::Album,
        secondary,
        year,
        credit(&[]),
    );
    outcome(
        track,
        group.clone(),
        plain_album(group),
        Proof::SameRecording,
    )
}

fn judged(
    question: NamingQuestion,
    verdict: NamingVerdict,
) -> HashMap<NamingQuestion, NamingJudgement> {
    HashMap::from([(
        question,
        NamingJudgement {
            verdict,
            reason: "Live recordings keep their marker.".into(),
            by: NamingJudge::Llm {
                model: "claude:opus".into(),
                prompt_version: 1,
            },
            at: 1,
        },
    )])
}

#[test]
fn track_retitles_follow_the_titles_naming_verdict() {
    let rolling_stone = (
            subject(
                "Jimi Hendrix",
                "Like A Rolling Stone (Live)",
                "The Jimi Hendrix Experience (disc 1)",
                None,
            ),
            picked(
                "Like a Rolling Stone (live at the Monterey International Pop Festival, Los Angeles, CA, June 18, 1967)",
                "The Jimi Hendrix Experience",
                false,
                "2000-09-12",
            ),
        );
    let valerie = (
        subject(
            "Amy Winehouse",
            "Valerie - Live At BBC Radio 1 Live Lounge, London / 2007",
            "Back to Black (Deluxe Edition)",
            None,
        ),
        picked(
            "Valerie (live at Jo Whiley)",
            "Back to Black",
            false,
            "2006-10-27",
        ),
    );
    let who_loves_the_sun = (
        subject(
            "The Velvet Underground",
            "Who Loves The Sun",
            "Loaded: Re-Loaded 45th Anniversary Edition",
            None,
        ),
        picked(
            "Who Loves the Sun (2012 remaster)",
            "Loaded: Re-Loaded",
            false,
            "2015-10-30",
        ),
    );
    let mack_the_knife = (
        subject(
            "Ella Fitzgerald",
            "Mack the Knife [Live]",
            "Ella in Berlin: Mack the Knife",
            None,
        ),
        picked(
            "Mack the Knife",
            "Ella in Berlin: Mack the Knife",
            true,
            "1960",
        ),
    );
    let sugar_plum = (
        subject(
            "Bill Evans",
            "Sugar Plum - Live",
            "On a Monday Evening",
            None,
        ),
        picked("Sugar Plum", "On a Monday Evening", true, "2017-03-03"),
    );
    let target = |title: &str, album: &str| Some((target(title, album, None), true));

    use NamingVerdict::{Keep, MusicBrainz, Unsure};
    use RetitleJudge::{Llm, Reviewer};
    type Case<'a> = (
        &'a (Subject, Outcome),
        Option<NamingVerdict>,
        RetitleJudge,
        Option<(Target, bool)>,
    );
    let monterey = "Like a Rolling Stone (live at the Monterey International Pop Festival, Los Angeles, CA, June 18, 1967)";
    let cases: Vec<Case> = vec![
        (
            &rolling_stone,
            Some(MusicBrainz),
            Llm,
            target(monterey, "The Jimi Hendrix Experience"),
        ),
        (
            &rolling_stone,
            Some(Keep),
            Llm,
            target("Like A Rolling Stone (Live)", "The Jimi Hendrix Experience"),
        ),
        (
            &rolling_stone,
            Some(Unsure),
            Llm,
            target(monterey, "The Jimi Hendrix Experience"),
        ),
        (
            &rolling_stone,
            None,
            Reviewer,
            target(monterey, "The Jimi Hendrix Experience"),
        ),
        (
            &rolling_stone,
            None,
            Llm,
            target("Like A Rolling Stone (Live)", "The Jimi Hendrix Experience"),
        ),
        (
            &valerie,
            Some(MusicBrainz),
            Llm,
            target("Valerie (live at Jo Whiley)", "Back to Black"),
        ),
        (
            &valerie,
            Some(Keep),
            Llm,
            target(
                "Valerie - Live At BBC Radio 1 Live Lounge, London / 2007",
                "Back to Black",
            ),
        ),
        (
            &valerie,
            Some(Unsure),
            Llm,
            target("Valerie (live at Jo Whiley)", "Back to Black"),
        ),
        (
            &valerie,
            None,
            Reviewer,
            target("Valerie (live at Jo Whiley)", "Back to Black"),
        ),
        (
            &valerie,
            None,
            Llm,
            target(
                "Valerie - Live At BBC Radio 1 Live Lounge, London / 2007",
                "Back to Black",
            ),
        ),
        (
            &who_loves_the_sun,
            Some(MusicBrainz),
            Llm,
            target("Who Loves the Sun (2012 remaster)", "Loaded: Re-Loaded"),
        ),
        (
            &who_loves_the_sun,
            Some(Keep),
            Llm,
            target("Who Loves The Sun", "Loaded: Re-Loaded"),
        ),
        (
            &who_loves_the_sun,
            Some(Unsure),
            Llm,
            target("Who Loves the Sun (2012 remaster)", "Loaded: Re-Loaded"),
        ),
        (
            &who_loves_the_sun,
            None,
            Reviewer,
            target("Who Loves the Sun (2012 remaster)", "Loaded: Re-Loaded"),
        ),
        (
            &who_loves_the_sun,
            None,
            Llm,
            target("Who Loves The Sun", "Loaded: Re-Loaded"),
        ),
        (
            &mack_the_knife,
            Some(MusicBrainz),
            Llm,
            target("Mack the Knife", "Ella in Berlin: Mack the Knife"),
        ),
        (&mack_the_knife, Some(Keep), Llm, None),
        (
            &mack_the_knife,
            Some(Unsure),
            Llm,
            target("Mack the Knife", "Ella in Berlin: Mack the Knife"),
        ),
        (
            &mack_the_knife,
            None,
            Reviewer,
            target("Mack the Knife", "Ella in Berlin: Mack the Knife"),
        ),
        (&mack_the_knife, None, Llm, None),
        (
            &sugar_plum,
            Some(MusicBrainz),
            Llm,
            target("Sugar Plum", "On a Monday Evening"),
        ),
        (&sugar_plum, Some(Keep), Llm, None),
        (
            &sugar_plum,
            Some(Unsure),
            Llm,
            target("Sugar Plum", "On a Monday Evening"),
        ),
        (
            &sugar_plum,
            None,
            Reviewer,
            target("Sugar Plum", "On a Monday Evening"),
        ),
        (&sugar_plum, None, Llm, None),
    ];
    for ((subject, outcome), verdict, judge, expected) in cases {
        let question = NamingQuestion::track(&subject.artist, &subject.track, &outcome.title);
        let verdicts = verdict.map_or_else(HashMap::new, |verdict| judged(question, verdict));
        let actual = derive_edit(
            subject,
            outcome,
            Chosen::Adjudicated,
            &[],
            &MovePolicy::default(),
            Naming {
                verdicts: &verdicts,
                judge,
            },
        )
        .proposal
        .map(|p| (p.target, p.approval.requires_confirmation()));
        assert_eq!(
            actual, expected,
            "{} with {verdict:?}, {judge:?}",
            subject.track
        );
    }

    let (subject, outcome) = &valerie;
    let verdicts = judged(
        NamingQuestion::track(&subject.artist, &subject.track, &outcome.title),
        Keep,
    );
    let proposal = derive_edit(
        subject,
        outcome,
        Chosen::Adjudicated,
        &[],
        &MovePolicy::default(),
        Naming {
            verdicts: &verdicts,
            judge: Llm,
        },
    )
    .proposal
    .unwrap();
    assert_eq!(
            proposal.rationale,
            "Title: \"Valerie - Live At BBC Radio 1 Live Lounge, London / 2007\" kept (MusicBrainz titles it \"Valerie (live at Jo Whiley)\")\n\
             Naming: keep your title (claude:opus): Live recordings keep their marker.\n\
             Album: Back to Black (Album, 2006) -> Back to Black (Album, 2006) (edition title of the same release group)\n\
             Proof: the same MusicBrainz recording\n\
             Chosen among several MusicBrainz options"
        );
}

#[test]
fn a_track_naming_verdict_ends_the_walk_at_the_option_it_judged() {
    let subject = subject(
        "Ella Fitzgerald",
        "Mack the Knife [Live]",
        "Ella in Berlin: Mack the Knife",
        None,
    );
    let berlin = picked(
        "Mack the Knife",
        "Ella in Berlin: Mack the Knife",
        true,
        "1960",
    );
    let other = picked(
        "Mack the Knife (live, 1960-02-13: Deutschlandhalle, Berlin)",
        "Ella in Berlin: Mack the Knife",
        true,
        "1960",
    );
    let berlin_question =
        NamingQuestion::track("Ella Fitzgerald", "Mack the Knife [Live]", "Mack the Knife");
    let keep = judged(berlin_question.clone(), NamingVerdict::Keep);
    let none = HashMap::new();
    let cases = [
        ("kept", &keep, (berlin.title.clone(), None, vec![])),
        (
            "waiting",
            &none,
            (berlin.title.clone(), None, vec![berlin_question]),
        ),
    ];
    for (name, verdicts, expected) in cases {
        let actual = derive_first(
            &subject,
            vec![
                (&berlin, Chosen::Adjudicated),
                (&other, Chosen::Adjudicated),
            ],
            &[],
            &MovePolicy::default(),
            Naming {
                verdicts,
                judge: RetitleJudge::Llm,
            },
        )
        .map(|(outcome, derived)| {
            (
                outcome.title.clone(),
                derived.proposal.map(|p| p.target),
                derived.waiting,
            )
        });
        assert_eq!(actual, Some(expected), "{name}");
    }
}

#[test]
fn compilation_move_without_a_canonical_credit_keeps_the_album_artist_for_review() {
    let (subject, mut outcome) = cocaine_on_cream();
    outcome.canonical.as_mut().unwrap().group.credit = ArtistCredit::default();
    let proposal = derive_edit(
        &subject,
        &outcome,
        Chosen::Deterministic,
        &[],
        &MovePolicy::default(),
        Naming::reviewer(),
    )
    .proposal
    .unwrap();
    assert_eq!(
        (proposal.target, proposal.approval.requires_confirmation()),
        (target("Cocaine", "Slowhand", Some("Various Artists")), true)
    );
}

#[test]
fn moves_credit_the_album_to_its_main_artists_in_the_users_spelling() {
    let cases = [
        (
            "Luis Fonsi",
            "Despacito",
            credit(&[
                ("luis-fonsi", "Luis Fonsi", " ft. "),
                ("daddy-yankee", "Daddy Yankee", ""),
            ]),
            "Luis Fonsi",
        ),
        (
            "Wiz Khalifa",
            "See You Again",
            credit(&[
                ("wiz-khalifa", "Wiz Khalifa", " feat. "),
                ("charlie-puth", "Charlie Puth", ""),
            ]),
            "Wiz Khalifa",
        ),
        (
            "Simon & Garfunkel",
            "The Sound of Silence",
            credit(&[
                ("paul-simon", "Simon", " & "),
                ("art-garfunkel", "Garfunkel", ""),
            ]),
            "Simon & Garfunkel",
        ),
        (
            "JAY-Z",
            "Empire State of Mind",
            credit(&[
                ("jay-z", "Jay\u{2010}Z", " & "),
                ("alicia-keys", "Alicia Keys", ""),
            ]),
            "JAY-Z & Alicia Keys",
        ),
        (
            "Simon and Garfunkel",
            "Mrs. Robinson",
            credit(&[
                ("paul-simon", "Simon", " & "),
                ("art-garfunkel", "Garfunkel", ""),
            ]),
            "Simon and Garfunkel",
        ),
    ];
    for (artist, track, canonical_credit, expected) in cases {
        let subject = subject(
            artist,
            track,
            "Now That's What I Call Music!",
            Some("Various Artists"),
        );
        let now = group(
            "now",
            "Now That's What I Call Music!",
            PrimaryType::Album,
            &[SecondaryType::Compilation],
            "2017",
            credit(&[(VARIOUS, "Various Artists", "")]),
        );
        let canonical = Canonical {
            group: group(
                track,
                track,
                PrimaryType::Single,
                &[],
                "2017",
                canonical_credit,
            ),
            rung: Rung::Single,
            single_first: false,
        };
        let outcome = outcome(track, now, Some(canonical), Proof::SameRecording);
        let proposal = derive_edit(
            &subject,
            &outcome,
            Chosen::Deterministic,
            &[],
            &MovePolicy::default(),
            Naming::reviewer(),
        )
        .proposal
        .unwrap();
        assert_eq!(
            proposal.target,
            target(track, track, Some(expected)),
            "{track}"
        );
    }
}

#[test]
fn move_policy_gates_each_kind_of_album_move() {
    let (cocaine, cocaine_outcome) = cocaine_on_cream();
    let holly = || credit(&[("buddy-holly", "Buddy Holly", "")]);
    let everyday = subject("Buddy Holly", "Everyday", "Peggy Sue", None);
    let everyday_outcome = outcome(
        "Everyday",
        group(
            "peggy-sue",
            "Peggy Sue",
            PrimaryType::Single,
            &[],
            "1957-09-20",
            holly(),
        ),
        plain_album(group(
            "buddy-holly",
            "Buddy Holly",
            PrimaryType::Album,
            &[],
            "1958-02-20",
            holly(),
        )),
        Proof::SameRecording,
    );
    let compilations = |mode| MovePolicy {
        compilations: mode,
        ..MovePolicy::default()
    };
    let singles = |mode| MovePolicy {
        singles: mode,
        ..MovePolicy::default()
    };
    let cases = [
        (
            &cocaine,
            &cocaine_outcome,
            compilations(MoveMode::Off),
            None,
        ),
        (
            &cocaine,
            &cocaine_outcome,
            compilations(MoveMode::Review),
            Some(true),
        ),
        (
            &cocaine,
            &cocaine_outcome,
            compilations(MoveMode::Auto),
            Some(false),
        ),
        (&everyday, &everyday_outcome, singles(MoveMode::Off), None),
        (
            &everyday,
            &everyday_outcome,
            singles(MoveMode::Auto),
            Some(true),
        ),
    ];
    for (subject, outcome, policy, expected) in cases {
        let actual = derive_edit(
            subject,
            outcome,
            Chosen::Deterministic,
            &[],
            &policy,
            Naming::reviewer(),
        )
        .proposal
        .map(|p| p.approval.requires_confirmation());
        assert_eq!(actual, expected, "{} with {policy:?}", subject.track);
    }
}

/// `None` means no proposal; otherwise the target and whether it needs review.
#[test]
fn proposals_and_their_approval() {
    let (cocaine, cocaine_outcome) = cocaine_on_cream();
    let holly = || credit(&[("buddy-holly", "Buddy Holly", "")]);
    let buddy_holly_album = group(
        "buddy-holly",
        "Buddy Holly",
        PrimaryType::Album,
        &[],
        "1958-02-20",
        holly(),
    );
    let peggy_sue_single = group(
        "peggy-sue",
        "Peggy Sue",
        PrimaryType::Single,
        &[],
        "1957-09-20",
        holly(),
    );
    let stones = || credit(&[("rolling-stones", "The Rolling Stones", "")]);
    let sticky_fingers = group(
        "4a4213d0-f224-3d51-bcca-476f61162681",
        "Sticky Fingers",
        PrimaryType::Album,
        &[],
        "1971-04-23",
        stones(),
    );
    let hot_rocks = group(
        "6e33402c-62db-3dc9-af86-2c1bef99ce2f",
        "Hot Rocks 1964–1971",
        PrimaryType::Album,
        &[SecondaryType::Compilation],
        "1971-12-20",
        stones(),
    );

    type Case = (
        &'static str,
        Subject,
        Outcome,
        Chosen,
        Vec<Target>,
        Option<(Target, bool)>,
    );
    let cases: Vec<Case> = vec![
        (
            "edition title within Slowhand's own release group",
            subject(
                "Eric Clapton",
                "Cocaine",
                "Slowhand 35th Anniversary (Super Deluxe)",
                None,
            ),
            outcome(
                "Cocaine",
                slowhand(),
                plain_album(slowhand()),
                Proof::SameRecording,
            ),
            Chosen::Deterministic,
            vec![],
            Some((target("Cocaine", "Slowhand", None), false)),
        ),
        (
            "live release group stays",
            subject(
                "Wes Montgomery",
                "Four on Six",
                "Smokin' at the Half Note",
                None,
            ),
            outcome(
                "Four on Six",
                smokin_at_the_half_note(),
                plain_album(incredible_jazz_guitar()),
                Proof::SameWork,
            ),
            Chosen::Deterministic,
            vec![],
            None,
        ),
        (
            "packaging-only title on a plain album",
            subject(
                "The Rolling Stones",
                "Wild Horses - 2009 Re-Mastered Digital Version",
                "Sticky Fingers",
                None,
            ),
            outcome(
                "Wild Horses",
                sticky_fingers.clone(),
                plain_album(sticky_fingers.clone()),
                Proof::SameRecording,
            ),
            Chosen::Deterministic,
            vec![],
            Some((
                Target {
                    title: "Wild Horses".to_string(),
                    album: Some("Sticky Fingers".to_string()),
                    album_artist: None,
                },
                false,
            )),
        ),
        (
            "packaging stripped and moved off a compilation together",
            subject(
                "The Rolling Stones",
                "Wild Horses - 2009 Re-Mastered Digital Version",
                "Hot Rocks 1964-1971",
                None,
            ),
            outcome(
                "Wild Horses",
                hot_rocks,
                plain_album(sticky_fingers),
                Proof::SameRecording,
            ),
            Chosen::Deterministic,
            vec![],
            Some((
                target("Wild Horses", "Sticky Fingers", Some("The Rolling Stones")),
                false,
            )),
        ),
        (
            "single to album needs review",
            subject("Buddy Holly", "Everyday", "Peggy Sue", None),
            outcome(
                "Everyday",
                peggy_sue_single.clone(),
                plain_album(buddy_holly_album.clone()),
                Proof::SameRecording,
            ),
            Chosen::Deterministic,
            vec![],
            Some((target("Everyday", "Buddy Holly", Some("Buddy Holly")), true)),
        ),
        (
            "identity marker is never stripped deterministically",
            subject(
                "Buddy Holly",
                "Everyday - Single Version",
                "Buddy Holly",
                None,
            ),
            outcome(
                "Everyday",
                buddy_holly_album.clone(),
                plain_album(buddy_holly_album.clone()),
                Proof::SameRecording,
            ),
            Chosen::Deterministic,
            vec![],
            None,
        ),
        (
            "an adjudicated title change is proposed for review",
            subject(
                "Buddy Holly",
                "Everyday - Single Version",
                "Buddy Holly",
                None,
            ),
            outcome(
                "Everyday",
                buddy_holly_album.clone(),
                plain_album(buddy_holly_album),
                Proof::SameRecording,
            ),
            Chosen::Adjudicated,
            vec![],
            Some((
                Target {
                    title: "Everyday".to_string(),
                    album: Some("Buddy Holly".to_string()),
                    album_artist: None,
                },
                true,
            )),
        ),
        (
            "adjudicated compilation move needs review",
            cocaine.clone(),
            cocaine_outcome.clone(),
            Chosen::Adjudicated,
            vec![],
            Some((target("Cocaine", "Slowhand", Some("Eric Clapton")), true)),
        ),
        (
            "same work without an ISRC link needs review",
            cocaine.clone(),
            Outcome {
                proof: Proof::SameWork,
                ..cocaine_outcome.clone()
            },
            Chosen::Deterministic,
            vec![],
            Some((target("Cocaine", "Slowhand", Some("Eric Clapton")), true)),
        ),
        (
            "ISRC duplicates more than 5 s apart need review",
            cocaine.clone(),
            Outcome {
                proof: shared_isrc(5_001),
                ..cocaine_outcome.clone()
            },
            Chosen::Deterministic,
            vec![],
            Some((target("Cocaine", "Slowhand", Some("Eric Clapton")), true)),
        ),
        (
            "an album preceded by its single needs review",
            cocaine.clone(),
            Outcome {
                canonical: Some(Canonical {
                    group: slowhand(),
                    rung: Rung::PlainAlbum,
                    single_first: true,
                }),
                ..cocaine_outcome.clone()
            },
            Chosen::Deterministic,
            vec![],
            Some((target("Cocaine", "Slowhand", Some("Eric Clapton")), true)),
        ),
        (
            "an album whose first release may not carry the recording needs review",
            cocaine.clone(),
            Outcome {
                canonical: Some(Canonical {
                    group: slowhand(),
                    rung: Rung::UndatedAlbum,
                    single_first: false,
                }),
                ..cocaine_outcome.clone()
            },
            Chosen::Deterministic,
            vec![],
            Some((target("Cocaine", "Slowhand", Some("Eric Clapton")), true)),
        ),
        (
            "a researched outcome needs review even when settled",
            cocaine.clone(),
            Outcome {
                proof: Proof::Researched {
                    by: "claude:opus".to_string(),
                },
                ..cocaine_outcome.clone()
            },
            Chosen::Deterministic,
            vec![],
            Some((target("Cocaine", "Slowhand", Some("Eric Clapton")), true)),
        ),
        (
            "a researched match never rewrites a title beyond packaging",
            subject(
                "Eric Clapton",
                "Cocaine (feat. J.J. Cale)",
                "The Cream of Clapton",
                None,
            ),
            Outcome {
                proof: Proof::Researched {
                    by: "claude:opus".to_string(),
                },
                ..cocaine_outcome.clone()
            },
            Chosen::Adjudicated,
            vec![],
            Some((
                target(
                    "Cocaine (feat. J.J. Cale)",
                    "Slowhand",
                    Some("Eric Clapton"),
                ),
                true,
            )),
        ),
        (
            "a rejected target is not proposed again",
            cocaine,
            cocaine_outcome,
            Chosen::Deterministic,
            vec![target("Cocaine", "Slowhand", Some("Eric Clapton"))],
            None,
        ),
    ];

    for (name, subject, outcome, chosen, rejected, expected) in cases {
        let actual = derive_edit(
            &subject,
            &outcome,
            chosen,
            &rejected,
            &MovePolicy::default(),
            Naming::reviewer(),
        )
        .proposal
        .map(|p| (p.target, p.approval.requires_confirmation()));
        assert_eq!(actual, expected, "{name}");
    }
}

impl Naming<'static> {
    /// No LLM and no verdicts.
    pub(crate) fn reviewer() -> Self {
        Self {
            verdicts: Box::leak(Box::default()),
            judge: RetitleJudge::Reviewer,
        }
    }
}

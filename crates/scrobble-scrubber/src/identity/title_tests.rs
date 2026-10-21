use super::IdentityMarker as I;
use super::Packaging as P;
use super::*;

fn parsed(
    raw: &str,
    base: &str,
    packaging: Vec<P>,
    identity: Vec<I>,
    credits: &[&str],
) -> ParsedTitle {
    ParsedTitle {
        raw: raw.to_string(),
        base: base.to_string(),
        packaging,
        identity,
        credits: credits.iter().map(|c| c.to_string()).collect(),
    }
}

fn other(note: &str) -> I {
    I::OtherVersion(note.to_string())
}

fn remaster(year: u16) -> P {
    P::Remaster { year: Some(year) }
}

#[test]
fn parses_real_scrobbled_titles() {
    let cases = [
        (
            "Wild Horses - 2009 Re-Mastered Digital Version",
            "Wild Horses",
            vec![remaster(2009)],
            vec![],
            &[][..],
        ),
        (
            "Something - Remastered 2015",
            "Something",
            vec![remaster(2015)],
            vec![],
            &[],
        ),
        (
            "Exercise One - 2007 Re-mastered",
            "Exercise One",
            vec![remaster(2007)],
            vec![],
            &[],
        ),
        (
            "Blue Rondo a' la Turk (Digitally Re-Mastered 2009)",
            "Blue Rondo a' la Turk",
            vec![remaster(2009)],
            vec![],
            &[],
        ),
        (
            "Paranoid - 2012",
            "Paranoid",
            vec![remaster(2012)],
            vec![],
            &[],
        ),
        (
            "The Saga Of Pepote Rouge - 2000 Digital",
            "The Saga Of Pepote Rouge",
            vec![remaster(2000)],
            vec![],
            &[],
        ),
        (
            "Bring It on Home - Remaster",
            "Bring It on Home",
            vec![P::Remaster { year: None }],
            vec![],
            &[],
        ),
        (
            "Real Love - 1996 Mix",
            "Real Love",
            vec![],
            vec![other("1996 Mix")],
            &[],
        ),
        (
            "Gonorrhea - Explicit Version",
            "Gonorrhea",
            vec![P::Explicit],
            vec![],
            &[],
        ),
        (
            "Star In Da Hood - explicit album version",
            "Star In Da Hood",
            vec![P::Explicit, P::AlbumVersion],
            vec![],
            &[],
        ),
        (
            "Spinal Cords - Explicit Bonus Version",
            "Spinal Cords",
            vec![P::Explicit, P::BonusTrack],
            vec![],
            &[],
        ),
        ("Hey Ya! (Clean)", "Hey Ya!", vec![], vec![I::Clean], &[]),
        (
            "Gold Digger - Amended Album Version",
            "Gold Digger",
            vec![P::AlbumVersion],
            vec![I::Clean],
            &[],
        ),
        (
            "Hit 'Em Up - Single Version (Edited)",
            "Hit 'Em Up",
            vec![],
            vec![I::SingleVersion, I::Clean],
            &[],
        ),
        (
            "Everyday - Single Version",
            "Everyday",
            vec![],
            vec![I::SingleVersion],
            &[],
        ),
        (
            "Used to Be - 2008 single version",
            "Used to Be",
            vec![],
            vec![I::SingleVersion],
            &[],
        ),
        (
            "Hit 'Em Up - Single Version (Explicit)",
            "Hit 'Em Up",
            vec![P::Explicit],
            vec![I::SingleVersion],
            &[],
        ),
        (
            "Personal Jesus - Original Single Version",
            "Personal Jesus",
            vec![],
            vec![I::SingleVersion],
            &[],
        ),
        (
            "Cold Turkey - Single Version / Remastered 2010",
            "Cold Turkey",
            vec![remaster(2010)],
            vec![I::SingleVersion],
            &[],
        ),
        (
            "the man who sold the world - live",
            "the man who sold the world",
            vec![],
            vec![I::Live],
            &[],
        ),
        (
            "Look At Me Now (feat. Lil Wayne & Busta Rhymes)",
            "Look At Me Now",
            vec![],
            vec![],
            &["Lil Wayne & Busta Rhymes"],
        ),
        (
            "Look At Me Now featuring Lil Wayne & Busta Rhymes - Explicit Version",
            "Look At Me Now",
            vec![P::Explicit],
            vec![],
            &["Lil Wayne & Busta Rhymes"],
        ),
        (
            "You Don't Know Me (featuring Regina Spektor) - Explicit Album Version",
            "You Don't Know Me",
            vec![P::Explicit, P::AlbumVersion],
            vec![],
            &["Regina Spektor"],
        ),
        (
            "Ice Cream (with Selena Gomez)",
            "Ice Cream",
            vec![],
            vec![],
            &["Selena Gomez"],
        ),
        (
            "Quit Playing Games (With My Heart)",
            "Quit Playing Games (With My Heart)",
            vec![],
            vec![],
            &[],
        ),
        (
            "History Eraser - Original Mix",
            "History Eraser",
            vec![],
            vec![other("Original Mix")],
            &[],
        ),
        (
            "Jambalaya (On The Bayou) - Original",
            "Jambalaya (On The Bayou)",
            vec![],
            vec![other("Original")],
            &[],
        ),
        (
            "Love Story (Taylor's Version)",
            "Love Story",
            vec![],
            vec![other("Taylor's Version")],
            &[],
        ),
        (
            "Lover Man - Live at the Academy of Music / 1971 / Soundboard Mix",
            "Lover Man",
            vec![],
            vec![I::Live],
            &[],
        ),
        (
            "Fire - Live at Warfield Theatre, San Francisco, CA - December 1980",
            "Fire",
            vec![],
            vec![I::Live],
            &[],
        ),
        (
            "So What - Live - (Take 2)",
            "So What",
            vec![],
            vec![I::Live, other("Take 2")],
            &[],
        ),
        (
            "Personal Jesus - Steven Wilson Stereo Remix",
            "Personal Jesus",
            vec![],
            vec![other("Steven Wilson Stereo Remix")],
            &[],
        ),
        (
            "Elevator Love Letter - Sadlands Demo",
            "Elevator Love Letter",
            vec![],
            vec![other("Sadlands Demo")],
            &[],
        ),
        (
            "Hey Ya! - UK Radio Edit",
            "Hey Ya!",
            vec![],
            vec![other("UK Radio Edit")],
            &[],
        ),
        (
            "Paint It Black - Mono Version",
            "Paint It Black",
            vec![],
            vec![I::Mono],
            &[],
        ),
        (
            "Here Comes the Sun - Album Version (Stereo)",
            "Here Comes the Sun",
            vec![P::AlbumVersion],
            vec![I::Stereo],
            &[],
        ),
        (
            "Twist and Shout (Instr.)",
            "Twist and Shout",
            vec![],
            vec![I::Instrumental],
            &[],
        ),
        (
            "The Imperial March - Darth Vader's Theme",
            "The Imperial March - Darth Vader's Theme",
            vec![],
            vec![],
            &[],
        ),
        (
            "Soul Power - Pt.1 / Single Version",
            "Soul Power - Pt.1 / Single Version",
            vec![],
            vec![],
            &[],
        ),
        (
            "Peggy Sue Got Married - Single Version (No Overdubs)",
            "Peggy Sue Got Married - Single Version (No Overdubs)",
            vec![],
            vec![],
            &[],
        ),
        (
            "Good Kid (Interlude)",
            "Good Kid (Interlude)",
            vec![],
            vec![],
            &[],
        ),
        (
            "(Forgive Me) My Little Flower Princess - 2010",
            "(Forgive Me) My Little Flower Princess",
            vec![remaster(2010)],
            vec![],
            &[],
        ),
        ("(Live)", "(Live)", vec![], vec![], &[]),
    ];
    for (raw, base, packaging, identity, credits) in cases {
        assert_eq!(
            parse_title(raw),
            parsed(raw, base, packaging, identity, credits),
            "{raw}"
        );
    }
}

#[test]
fn qualified_edit_remix_and_demo_markers_stay_verbatim() {
    let cases = [
        ("Levels - Radio Edit", vec![I::Edit]),
        ("Levels - Extended Edit", vec![other("Extended Edit")]),
        ("Silence - Tiësto Edit", vec![other("Tiësto Edit")]),
        ("Strawberry Fields Forever - Remix", vec![I::Remix]),
        (
            "Strawberry Fields Forever - 2015 Remix",
            vec![other("2015 Remix")],
        ),
        (
            "Strawberry Fields Forever - 2017 Remix",
            vec![other("2017 Remix")],
        ),
        ("Elevator Love Letter - Demo", vec![I::Demo]),
        ("Elevator Love Letter - Home Demo", vec![other("Home Demo")]),
    ];
    for (raw, identity) in cases {
        assert_eq!(parse_title(raw).identity, identity, "{raw}");
    }
}

#[test]
fn edition_suffixes_need_known_edition_words() {
    let cases = [
        (
            "MTV Unplugged (Acoustic Edition)",
            "MTV Unplugged (Acoustic Edition)",
        ),
        ("Hysteria (Live Edition)", "Hysteria (Live Edition)"),
        ("Rumours (Drumless Edition)", "Rumours (Drumless Edition)"),
        (
            "Origin of Symmetry (Wembley Edition)",
            "Origin of Symmetry (Wembley Edition)",
        ),
        ("Hysteria (Collector's Edition)", "Hysteria"),
        ("Rumours (Super Deluxe Edition)", "Rumours"),
        (
            "Origin of Symmetry (XX Anniversary RemiXX)",
            "Origin of Symmetry (XX Anniversary RemiXX)",
        ),
        ("Kind of Blue (Limited Edition)", "Kind of Blue"),
    ];
    for (album, stripped) in cases {
        assert_eq!(strip_edition(album), stripped, "{album}");
    }
}

#[test]
fn edition_noise_naming_a_single_or_ep_is_reported() {
    let cases = [
        ("Purple Rain - Single", true),
        ("Come As You Are - EP", true),
        ("Purple Rain (Deluxe Edition)", false),
        ("Purple Rain", false),
        (
            "The Beat Generation 10th Anniversary Single Collection",
            false,
        ),
    ];
    for (album, names_single) in cases {
        assert_eq!(edition_names_single_or_ep(album), names_single, "{album}");
    }
}

#[test]
fn strips_album_edition_noise() {
    let cases = [
        ("Slowhand 35th Anniversary (Super Deluxe)", "Slowhand"),
        ("Sound & Color (Deluxe Edition)", "Sound & Color"),
        ("Come As You Are - Single", "Come As You Are"),
        (
            "Sublime: 10th Anniversary Deluxe Edition (disc 2)",
            "Sublime",
        ),
        ("AH UM - 50th Anniversary (Legacy Edition)", "AH UM"),
        (
            "Songs in A Minor (10th Anniversary Edition) (Deluxe Edition)",
            "Songs in A Minor",
        ),
        (
            "Turn On the Bright Lights: The Tenth Anniversary Edition",
            "Turn On the Bright Lights",
        ),
        ("Atlantic Crossing [Deluxe Edition]", "Atlantic Crossing"),
        (
            "My Generation (50th Anniversary / Super Deluxe)",
            "My Generation",
        ),
        ("Megalithic Symphony Deluxe", "Megalithic Symphony"),
        ("Blue Train (The Rudy Van Gelder Edition)", "Blue Train"),
        ("Sticky Fingers (Remastered)", "Sticky Fingers"),
        ("Blurryface (Explicit Version)", "Blurryface"),
        (
            "Waiting For Columbus [Live Deluxe]",
            "Waiting For Columbus [Live Deluxe]",
        ),
        ("Red (Taylor's Version)", "Red (Taylor's Version)"),
        ("Smokin' at the Half Note", "Smokin' at the Half Note"),
        ("1999", "1999"),
        ("Deluxe", "Deluxe"),
        (
            "The Beat Generation 10th Anniversary Single Collection",
            "The Beat Generation 10th Anniversary Single Collection",
        ),
    ];
    for (album, stripped) in cases {
        assert_eq!(strip_edition(album), stripped, "{album}");
    }
}

#[test]
fn folds_typographic_and_article_variants() {
    assert_eq!(fold("Hot Rocks 1964–1971"), fold("Hot Rocks 1964-1971"));
    assert_eq!(fold("Smokin’ at the Half Note"), "smokin' at the half note");
    assert_eq!(fold("Sound & Color"), "sound and color");
    assert_eq!(
        fold("The  Man Who Sold  the World"),
        "man who sold the world"
    );
    assert_eq!(fold("ＷＩＬＤ\u{a0}HORSES"), "wild horses");
    assert_eq!(fold("The"), "the");
    assert_eq!(
        fold("To Ü (feat. AlunaGeorge) [Remixes]"),
        fold("To Ü (feat. AlunaGeorge) (Remixes)")
    );
}

#[test]
fn classifies_title_changes_against_the_mb_title() {
    let cases = [
        (
            "Wild Horses - 2009 Re-Mastered Digital Version",
            "Wild Horses",
            TitleChange::PackagingOnly,
        ),
        (
            "Gonorrhea - Explicit Version",
            "Gonorrhea",
            TitleChange::PackagingOnly,
        ),
        (
            "Smokin’ at the Half Note",
            "Smokin' at the Half Note",
            TitleChange::None,
        ),
        ("Hey Ya! (Clean)", "Hey Ya!", TitleChange::Other),
        ("Hey Ya! (Censored Version)", "Hey Ya!", TitleChange::Other),
        ("Everyday - Single Version", "Everyday", TitleChange::Other),
        (
            "the man who sold the world - live",
            "The Man Who Sold the World",
            TitleChange::Other,
        ),
        (
            "Look At Me Now (feat. Lil Wayne & Busta Rhymes)",
            "Look at Me Now",
            TitleChange::Other,
        ),
        (
            "Something - Remastered 2015",
            "Something in the Way",
            TitleChange::Other,
        ),
        ("Cocaine", "Cocaine", TitleChange::None),
    ];
    for (raw, mb_title, change) in cases {
        assert_eq!(
            classify_title_change(&parse_title(raw), mb_title),
            change,
            "{raw}"
        );
    }
}

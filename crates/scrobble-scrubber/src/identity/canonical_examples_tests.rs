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

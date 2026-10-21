//! Pure title/album normalization: the single home of the junk taxonomy.

use unicode_normalization::UnicodeNormalization;

/// A parsed last.fm title. `base` is what MusicBrainz is searched with; it is never
/// written back to last.fm (only MB recording titles are).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ParsedTitle {
    pub raw: String,
    pub base: String,
    /// Discardable: never changes which recording this is.
    pub packaging: Vec<Packaging>,
    /// Names a distinct performance or mix: constrains candidates, never stripped from a
    /// title that reaches last.fm.
    pub identity: Vec<IdentityMarker>,
    /// The credited names from "(feat. X)", "ft. X", "(with X)", verbatim. Used only to
    /// widen artist matching.
    pub credits: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Packaging {
    /// "Remastered", "2009 Remaster", "Remastered 2015", "2009 Re-Mastered Digital Version",
    /// and a bare year suffix ("Paranoid - 2012").
    Remaster {
        year: Option<u16>,
    },
    Explicit,
    /// "Album Version", "LP Version".
    AlbumVersion,
    BonusTrack,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum IdentityMarker {
    Live,
    Remix,
    /// "Radio Edit", "Edit", "Radio Version".
    Edit,
    SingleVersion,
    Acoustic,
    Demo,
    Instrumental,
    Mono,
    Stereo,
    /// "Clean", "Edited", "Amended Version", "Censored": different audio from the
    /// explicit recording.
    Clean,
    /// Anything else that looks like a version note ("Original Mix", "Taylor's Version",
    /// "Take 2", "Peel Session"): opaque but identity-bearing.
    OtherVersion(String),
}

impl IdentityMarker {
    /// A remix, edit or demo, bare or qualified ("2015 Remix", "Extended Edit", "Home
    /// Demo").
    pub(crate) fn names_cut(&self) -> bool {
        match self {
            Self::Remix | Self::Edit | Self::Demo => true,
            Self::OtherVersion(note) => words(note)
                .iter()
                .any(|word| matches!(word.as_str(), "remix" | "rmx" | "edit" | "demo")),
            _ => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Marker {
    Packaging(Packaging),
    Identity(IdentityMarker),
    Credit(String),
}

const DASHES: [&str; 3] = [" - ", " – ", " — "];

/// Peels recognized " - X", "(X)", "[X]" suffixes and trailing "feat. X" credits right to
/// left. An unrecognized suffix stops the peel and stays in `base` (unknown suffixes are
/// content: "The Imperial March - Darth Vader's Theme").
/// Invariant: `base` is never empty; if peeling would empty it, `base == raw`.
pub(crate) fn parse_title(raw: &str) -> ParsedTitle {
    let mut rest = raw.trim();
    let mut peeled = Vec::new();
    while let Some((head, markers)) = peel_suffix(rest) {
        peeled.push(markers);
        rest = head;
    }

    let mut parsed = ParsedTitle {
        raw: raw.to_string(),
        base: if rest.is_empty() { raw } else { rest }.to_string(),
        packaging: Vec::new(),
        identity: Vec::new(),
        credits: Vec::new(),
    };
    for marker in peeled.into_iter().rev().flatten() {
        match marker {
            Marker::Packaging(p) => parsed.packaging.push(p),
            Marker::Identity(i) => parsed.identity.push(i),
            Marker::Credit(c) => parsed.credits.push(c),
        }
    }
    parsed
}

fn peel_suffix(s: &str) -> Option<(&str, Vec<Marker>)> {
    if let Some((head, inner)) = trailing_bracket(s) {
        let head = trim_dash(head);
        if head.is_empty() {
            return None;
        }
        return Some((head, classify_segment(inner, true)?));
    }
    if DASHES.iter().any(|dash| s.contains(dash)) {
        // Prefer the shortest recognized suffix, but let a wider one absorb dashes inside
        // it ("Live at Warfield Theatre, CA - December 1980").
        return dash_positions(s).find_map(|(pos, len)| {
            let head = s[..pos].trim_end();
            let markers = classify_segment(&s[pos + len..], false)?;
            (!head.is_empty()).then_some((head, markers))
        });
    }
    let (head, names) = inline_credit(s)?;
    Some((head, vec![Marker::Credit(names.to_string())]))
}

/// Byte positions and lengths of every dash separator, rightmost first.
fn dash_positions(s: &str) -> impl Iterator<Item = (usize, usize)> + '_ {
    let mut positions: Vec<(usize, usize)> = DASHES
        .iter()
        .flat_map(|dash| s.match_indices(dash).map(|(pos, d)| (pos, d.len())))
        .collect();
    positions.sort_unstable_by_key(|&(pos, _)| std::cmp::Reverse(pos));
    positions.into_iter()
}

fn trim_dash(s: &str) -> &str {
    let s = s.trim_end();
    DASHES
        .iter()
        .find_map(|dash| s.strip_suffix(dash.trim_end()))
        .map_or(s, str::trim_end)
}

/// `"Song (Live)"` -> `("Song", "Live")`, matching nested brackets.
fn trailing_bracket(s: &str) -> Option<(&str, &str)> {
    let close = s.chars().last()?;
    let open = match close {
        ')' => '(',
        ']' => '[',
        _ => return None,
    };
    let mut depth = 0usize;
    for (pos, c) in s.char_indices().rev() {
        if c == close {
            depth += 1;
        } else if c == open {
            depth -= 1;
            if depth == 0 {
                return Some((&s[..pos], &s[pos + 1..s.len() - 1]));
            }
        }
    }
    None
}

fn inline_credit(s: &str) -> Option<(&str, &str)> {
    let lower = s.to_lowercase();
    if lower.len() != s.len() {
        return None;
    }
    [" featuring ", " feat. ", " feat ", " ft. ", " ft "]
        .iter()
        .filter_map(|kw| lower.find(kw).map(|pos| (pos, kw.len())))
        .min()
        .map(|(pos, len)| (s[..pos].trim_end(), s[pos + len..].trim()))
        .filter(|(head, names)| !head.is_empty() && !names.is_empty())
}

fn credit_names(segment: &str, bracketed: bool) -> Option<&str> {
    // Lowercase "(with X)" is a credit; title-cased "(With My Heart)" is a subtitle.
    if bracketed {
        if let Some(names) = segment.strip_prefix("with ") {
            return Some(names.trim()).filter(|names| !names.is_empty());
        }
    }
    let lower = segment.to_lowercase();
    if lower.len() != segment.len() {
        return None;
    }
    ["featuring ", "feat. ", "feat ", "ft. ", "ft "]
        .iter()
        .find(|kw| lower.starts_with(*kw))
        .map(|kw| segment[kw.len()..].trim())
        .filter(|names| !names.is_empty())
}

fn classify_segment(segment: &str, bracketed: bool) -> Option<Vec<Marker>> {
    let segment = segment.trim();
    if let Some(names) = credit_names(segment, bracketed) {
        return Some(vec![Marker::Credit(names.to_string())]);
    }
    if is_live_note(&words(segment)) {
        return Some(vec![Marker::Identity(IdentityMarker::Live)]);
    }
    let parts: Vec<&str> = segment
        .split('/')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    if parts.is_empty() {
        return None;
    }
    let classified: Option<Vec<Vec<Marker>>> = parts
        .iter()
        .map(|part| {
            classify_words(part).or_else(|| version_note(part).map(|m| vec![Marker::Identity(m)]))
        })
        .collect();
    classified.map(|parts| parts.into_iter().flatten().collect())
}

fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .chars()
        .map(|c| match c {
            '\u{2019}' => '\'',
            c if c.is_alphanumeric() || c == '\'' || c == '-' => c,
            _ => ' ',
        })
        .collect::<String>()
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

fn year(word: &str) -> Option<u16> {
    let year: u16 = word.parse().ok()?;
    (word.len() == 4 && (1900..=2099).contains(&year)).then_some(year)
}

#[derive(PartialEq)]
enum Word {
    Remaster,
    Marker(Marker),
    Filler,
}

fn word_role(word: &str) -> Option<Word> {
    use IdentityMarker as I;
    let marker = match word {
        "remaster" | "remastered" | "re-master" | "re-mastered" | "remasterd" | "master"
        | "mastered" => return Some(Word::Remaster),
        "version" | "ver" | "digital" | "digitally" | "track" | "original" => {
            return Some(Word::Filler)
        }
        "explicit" => Marker::Packaging(Packaging::Explicit),
        "clean" | "edited" | "amended" | "censored" => Marker::Identity(I::Clean),
        "album" | "lp" => Marker::Packaging(Packaging::AlbumVersion),
        "bonus" => Marker::Packaging(Packaging::BonusTrack),
        "live" => Marker::Identity(I::Live),
        "remix" | "rmx" => Marker::Identity(I::Remix),
        "edit" | "radio" => Marker::Identity(I::Edit),
        "single" => Marker::Identity(I::SingleVersion),
        "acoustic" | "unplugged" => Marker::Identity(I::Acoustic),
        "demo" => Marker::Identity(I::Demo),
        "instrumental" | "instr" => Marker::Identity(I::Instrumental),
        "mono" => Marker::Identity(I::Mono),
        "stereo" => Marker::Identity(I::Stereo),
        _ => return None,
    };
    Some(Word::Marker(marker))
}

/// A part made only of known words ("2009 Re-Mastered Digital Version", "Explicit Album
/// Version", "Original Single Mono Version"). Years name a remaster unless the part also
/// names another marker ("Paranoid - 2012", "2000 Digital"; but "2008 single version").
/// A dated remix, edit or demo ("2015 Remix") is its own version, kept verbatim.
fn classify_words(part: &str) -> Option<Vec<Marker>> {
    let words = words(part);
    let mut years = Vec::new();
    let mut roles = Vec::new();
    for word in &words {
        match year(word) {
            Some(y) => years.push(y),
            None => roles.push(word_role(word)?),
        }
    }
    let names_cut = roles.iter().any(|role| {
        matches!(
            role,
            Word::Marker(Marker::Identity(
                IdentityMarker::Remix | IdentityMarker::Edit | IdentityMarker::Demo
            ))
        )
    });
    if names_cut && !years.is_empty() {
        return Some(vec![Marker::Identity(IdentityMarker::OtherVersion(
            part.trim().to_string(),
        ))]);
    }
    let first_year = years.first().copied();
    let mut markers = Vec::new();
    let names_marker = roles.iter().any(|role| matches!(role, Word::Marker(_)));
    if roles.contains(&Word::Remaster) || first_year.is_some() && !names_marker {
        markers.push(Marker::Packaging(Packaging::Remaster { year: first_year }));
    }
    for role in roles {
        if let Word::Marker(marker) = role {
            if !markers.contains(&marker) {
                markers.push(marker);
            }
        }
    }
    (!markers.is_empty()).then_some(markers)
}

fn is_live_note(words: &[String]) -> bool {
    words.iter().enumerate().any(|(i, word)| {
        word == "live"
            && (i == 0
                || i + 1 == words.len()
                || matches!(
                    words[i + 1].as_str(),
                    "at" | "in"
                        | "from"
                        | "on"
                        | "with"
                        | "version"
                        | "session"
                        | "sesh"
                        | "recording"
                        | "album"
                ))
    })
}

/// A part with unknown words that still reads as a version note, keyed on its head noun.
/// The unknown words qualify it ("Oliver Remix", "Sadlands Demo", "Extended Edit"), so it
/// has to match verbatim.
fn version_note(part: &str) -> Option<IdentityMarker> {
    let words = words(part);
    let (first, last) = (words.first()?.as_str(), words.last()?.as_str());
    let note = matches!(first, "take" | "recorded")
        || matches!(
            last,
            "remix"
                | "rmx"
                | "edit"
                | "demo"
                | "version"
                | "ver"
                | "mix"
                | "take"
                | "session"
                | "sessions"
                | "sesh"
                | "outtake"
                | "rehearsal"
                | "recording"
                | "re-recording"
                | "re-recorded"
                | "rerecorded"
                | "original"
        );
    note.then(|| IdentityMarker::OtherVersion(part.trim().to_string()))
}

/// Album strings with edition noise stripped ("(Deluxe Edition)", "(Remastered)",
/// "35th Anniversary (Super Deluxe)", " - Single", " - EP"), for search and matching only.
pub(crate) fn strip_edition(album: &str) -> String {
    split_edition(album).0.to_string()
}

/// Whether the noise [`strip_edition`] removes names a single or EP ("Purple Rain - Single").
pub(crate) fn edition_names_single_or_ep(album: &str) -> bool {
    words(split_edition(album).1)
        .iter()
        .any(|word| word == "single" || word == "ep")
}

/// `(head, edition noise)`.
fn split_edition(album: &str) -> (&str, &str) {
    let album = album.trim();
    let mut rest = album;
    while let Some(head) = peel_edition(rest) {
        rest = head;
    }
    (rest, &album[rest.len()..])
}

fn peel_edition(s: &str) -> Option<&str> {
    let head = if let Some((head, inner)) = trailing_bracket(s) {
        is_edition_note(inner).then_some(trim_dash(head))?
    } else if let Some((pos, len)) = [" - ", " – ", ": "]
        .iter()
        .filter_map(|sep| s.rfind(sep).map(|pos| (pos, sep.len())))
        .max()
    {
        is_edition_note(&s[pos + len..]).then_some(s[..pos].trim_end())?
    } else {
        trailing_edition_words(s)?
    };
    (!head.is_empty()).then_some(head)
}

const EDITION_ANCHORS: &[&str] = &[
    "deluxe",
    "expanded",
    "edition",
    "version",
    "special",
    "legacy",
    "anniversary",
    "remaster",
    "remastered",
    "reissue",
    "re-issue",
    "bonus",
    "standard",
    "complete",
    "explicit",
    "clean",
    "edited",
    "single",
    "ep",
    "release",
    "disc",
    "cd",
    "limited",
];

const EDITION_FILLERS: &[&str] = &[
    "super",
    "collector's",
    "collectors",
    "celebration",
    "track",
    "tracks",
    "the",
    "and",
    "w",
    "us",
    "u",
    "s",
    "uk",
    "international",
    "pa",
    "digital",
    "year",
    "tenth",
    "twentieth",
    "thirtieth",
    "fortieth",
    "fiftieth",
    "rudy",
    "van",
    "gelder",
    "rvg",
];

fn is_ordinal(word: &str) -> bool {
    let digits = word.trim_end_matches(char::is_alphabetic);
    !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && matches!(&word[digits.len()..], "st" | "nd" | "rd" | "th")
}

/// Every word is edition vocabulary, with at least one anchor. "Edition" alone isn't
/// enough: "(Acoustic Edition)" and "(Live Edition)" are different recordings.
fn is_edition_note(note: &str) -> bool {
    let words = words(note);
    let known = |w: &String| {
        EDITION_ANCHORS.contains(&w.as_str())
            || EDITION_FILLERS.contains(&w.as_str())
            || is_ordinal(w)
            || w.bytes().all(|b| b.is_ascii_digit())
    };
    words.iter().all(known) && words.iter().any(|w| EDITION_ANCHORS.contains(&w.as_str()))
}

/// Unbracketed trailing edition words: "Slowhand 35th Anniversary", "Megalithic Symphony
/// Deluxe". Narrower than [`is_edition_note`]: bare numbers and "Single" stay.
fn trailing_edition_words(s: &str) -> Option<&str> {
    const BARE: &[&str] = &[
        "deluxe",
        "super",
        "expanded",
        "edition",
        "special",
        "legacy",
        "anniversary",
        "remastered",
        "limited",
        "collector's",
    ];
    const STRONG: &[&str] = &["deluxe", "anniversary", "remastered"];

    let mut head = s.trim_end();
    let mut strong = false;
    while let Some((rest, word)) = head.rsplit_once(char::is_whitespace) {
        let word = word.to_lowercase();
        if !(BARE.contains(&word.as_str()) || is_ordinal(&word)) {
            break;
        }
        strong |= STRONG.contains(&word.as_str());
        head = rest.trim_end();
    }
    strong.then(|| trim_dash(head))
}

/// Equality key: NFKC, lowercase, unified quotes/dashes/whitespace, `&` == `and`,
/// leading "the " dropped. Fold-equal strings are never edited into each other.
pub(crate) fn fold(s: &str) -> String {
    let unified: String =
        s.nfkc()
            .flat_map(char::to_lowercase)
            .map(|c| match c {
                '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' | '\u{2032}' | '`'
                | '\u{00B4}' => '\'',
                '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{201F}' | '\u{2033}' => '"',
                '\u{2010}'..='\u{2015}' | '\u{2212}' => '-',
                '[' => '(',
                ']' => ')',
                c => c,
            })
            .collect::<String>()
            .replace('&', " and ");
    let words: Vec<&str> = unified.split_whitespace().collect();
    match words.split_first() {
        Some((&"the", rest)) if !rest.is_empty() => rest.join(" "),
        _ => words.join(" "),
    }
}

/// What a proposed title change consists of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TitleChange {
    None,
    /// The scrobbled title is the MB title plus packaging junk only (under `fold`).
    PackagingOnly,
    /// Anything else (different words, credits, typos). Never auto-applied.
    Other,
}

pub(crate) fn classify_title_change(parsed: &ParsedTitle, mb_title: &str) -> TitleChange {
    let mb = fold(mb_title);
    if fold(&parsed.raw) == mb {
        TitleChange::None
    } else if !parsed.packaging.is_empty()
        && parsed.identity.is_empty()
        && parsed.credits.is_empty()
        && fold(&parsed.base) == mb
    {
        TitleChange::PackagingOnly
    } else {
        TitleChange::Other
    }
}

#[cfg(test)]
#[path = "title_tests.rs"]
mod tests;

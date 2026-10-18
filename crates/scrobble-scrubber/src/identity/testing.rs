//! In-memory fakes for offline tests, and MusicBrainz fixture data taken from real
//! responses (MBIDs, titles, scores, lengths, ISRCs).

use super::mb::{
    ArtistCredit, CreditPart, Isrc, MbError, MbLookup, PartialDate, PrimaryType, RecordingDetail,
    RecordingHit, RecordingId, RecordingQuery, ReleaseGroup, ReleaseGroupId, ReleaseRef,
    SecondaryType, Truncated, WorkId, WorkLink,
};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::Duration;

/// Domain-value MusicBrainz. Unknown lookups are `NotFound`; unknown searches are empty.
#[derive(Default)]
pub(crate) struct FixtureMb {
    pub searches: HashMap<RecordingQuery, Vec<RecordingHit>>,
    /// Searches MB answers with more hits than it returned.
    pub truncated_searches: HashSet<RecordingQuery>,
    pub recordings: HashMap<RecordingId, RecordingDetail>,
    pub isrcs: HashMap<Isrc, Vec<RecordingId>>,
    pub works: HashMap<WorkId, Vec<RecordingId>>,
    pub releases: HashMap<RecordingId, Vec<ReleaseRef>>,
    /// Every call fails with `RateLimited` (after being recorded).
    pub rate_limited: bool,
    /// Every call, for asserting request patterns.
    pub calls: Mutex<Vec<String>>,
}

impl FixtureMb {
    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn record(&self, call: String) -> Result<(), MbError> {
        self.calls.lock().unwrap().push(call);
        if self.rate_limited {
            return Err(MbError::RateLimited(Duration::from_secs(30)));
        }
        Ok(())
    }

    pub fn with_search(
        mut self,
        title: &str,
        artist: &str,
        release: Option<&str>,
        hits: Vec<RecordingHit>,
    ) -> Self {
        self.searches.insert(query(title, artist, release), hits);
        self
    }

    pub fn with_recording(mut self, detail: RecordingDetail) -> Self {
        self.recordings.insert(detail.id.clone(), detail);
        self
    }

    pub fn with_isrc(mut self, isrc: &str, members: &[&str]) -> Self {
        self.isrcs
            .insert(Isrc(isrc.into()), members.iter().map(|m| rid(m)).collect());
        self
    }

    pub fn with_work(mut self, work: &str, recordings: &[&str]) -> Self {
        self.works.insert(
            WorkId(work.into()),
            recordings.iter().map(|r| rid(r)).collect(),
        );
        self
    }

    pub fn with_releases(mut self, recording: &str, releases: Vec<ReleaseRef>) -> Self {
        self.releases.insert(rid(recording), releases);
        self
    }

    /// Eric Clapton "Cocaine": the Cream of Clapton track (`aca688be`) is an unmerged
    /// duplicate of Slowhand's (`40ae4c06`) sharing ISRC NLF057790024; the 5.1 mix
    /// (`c2ee8d1d`) is the top hit when anchoring on "Slowhand".
    pub fn cocaine(self) -> Self {
        let slowhand_search = vec![
            hit(
                COCAINE_51,
                100,
                "Cocaine",
                "5.1 mix",
                222_000,
                clapton(),
                vec![rel(&slowhand())],
            ),
            hit(
                "426a6973-9583-4761-8cd1-efab138a98f5",
                82,
                "Cocaine",
                "",
                532_000,
                clapton(),
                vec![rel(&slowhand_at_70())],
            ),
            hit(
                COCAINE_STUDIO,
                66,
                "Cocaine",
                "original studio mix",
                216_640,
                clapton(),
                vec![rel(&cream_of_clapton()), rel(&slowhand())],
            ),
        ];
        self.with_search(
            "Cocaine",
            "Eric Clapton",
            Some("The Cream of Clapton"),
            vec![
                hit(
                    COCAINE_CREAM,
                    100,
                    "Cocaine",
                    "",
                    218_747,
                    clapton(),
                    vec![rel(&cream_of_clapton())],
                ),
                hit(
                    COCAINE_STUDIO,
                    56,
                    "Cocaine",
                    "original studio mix",
                    216_640,
                    clapton(),
                    vec![rel(&cream_of_clapton()), rel(&slowhand())],
                ),
            ],
        )
        .with_search("Cocaine", "Eric Clapton", Some("Slowhand"), slowhand_search)
        .with_recording(detail(
            COCAINE_CREAM,
            "Cocaine",
            "",
            218_747,
            clapton(),
            &["NLF057790024"],
            vec![work(COCAINE_WORK, &[])],
        ))
        .with_recording(detail(
            COCAINE_STUDIO,
            "Cocaine",
            "original studio mix",
            216_640,
            clapton(),
            &["GBA077700020", "NLF057790024", "USRY20517111"],
            vec![work(COCAINE_WORK, &["cover"])],
        ))
        .with_isrc("NLF057790024", &[COCAINE_CREAM, COCAINE_STUDIO])
        .with_work(COCAINE_WORK, &[COCAINE_CREAM, COCAINE_STUDIO])
        .with_releases(COCAINE_CREAM, vec![rel(&cream_of_clapton())])
        .with_releases(
            COCAINE_STUDIO,
            vec![
                rel(&slowhand()),
                rel(&cream_of_clapton()),
                rel(&group(
                    "f452a50f-96f8-3294-a023-af03c06b1203",
                    "Bad News Bears",
                    PrimaryType::Album,
                    &[SecondaryType::Soundtrack],
                    2005,
                    various_artists(),
                )),
            ],
        )
    }

    /// The Rolling Stones "Wild Horses" anchored on Sticky Fingers: two studio recordings
    /// on the plain album, plus acoustic, live and remix-mix variants that must not match.
    pub fn wild_horses(self) -> Self {
        self.with_search(
            "Wild Horses",
            "The Rolling Stones",
            Some("Sticky Fingers"),
            vec![
                hit(
                    "5a6062a4-c790-4c29-84c3-080ea9e332ef",
                    100,
                    "Wild Horses",
                    "",
                    344_000,
                    stones(),
                    vec![rel(&sticky_fingers())],
                ),
                hit(
                    "46fc6372-a9ed-40bc-8b7a-3bb24525255b",
                    100,
                    "Wild Horses",
                    "",
                    340_000,
                    stones(),
                    vec![rel(&sticky_fingers())],
                ),
                hit(
                    "4c349f51-3cf5-40e5-90df-e2a80f687080",
                    96,
                    "Wild Horses (acoustic)",
                    "",
                    347_000,
                    stones(),
                    vec![rel(&sticky_fingers())],
                ),
                hit(
                    "f4522df2-a77f-4ed9-820f-6d07633c0091",
                    96,
                    "Wild Horses",
                    "",
                    281_533,
                    stones(),
                    vec![rel(&group(
                        "d0a415d4-0d68-40d2-ae04-bfdfcc2b6623",
                        "Sticky Fingers Live",
                        PrimaryType::Album,
                        &[SecondaryType::Live],
                        2017,
                        stones(),
                    ))],
                ),
            ],
        )
    }

    /// "Wild Horses" on the Honk compilation, where two candidate recordings disagree:
    /// the 2009 mix (`c02a915c`, ISRC-linked to the original single master `1674d24d`) is
    /// also on Sticky Fingers, while `3afa1e36` is only on compilations.
    pub fn wild_horses_on_honk(self) -> Self {
        self.with_search(
            "Wild Horses",
            "The Rolling Stones",
            Some("Honk"),
            vec![
                hit(
                    WILD_HORSES_2009,
                    100,
                    "Wild Horses",
                    "2009 mix",
                    343_000,
                    stones(),
                    vec![rel(&honk()), rel(&sticky_fingers())],
                ),
                hit(
                    WILD_HORSES_COMP,
                    85,
                    "Wild Horses",
                    "",
                    342_000,
                    stones(),
                    vec![rel(&honk())],
                ),
            ],
        )
        .with_recording(detail(
            WILD_HORSES_2009,
            "Wild Horses",
            "2009 mix",
            343_000,
            stones(),
            &["GBUM70909466"],
            vec![],
        ))
        .with_recording(detail(
            WILD_HORSES_SINGLE,
            "Wild Horses",
            "original single stereo version",
            343_333,
            stones(),
            &[
                "USA176110020",
                "GBUM70909466",
                "USA177110020",
                "GBCJN7100003",
            ],
            vec![],
        ))
        .with_recording(detail(
            WILD_HORSES_COMP,
            "Wild Horses",
            "",
            342_000,
            stones(),
            &[],
            vec![],
        ))
        .with_isrc("GBUM70909466", &[WILD_HORSES_2009, WILD_HORSES_SINGLE])
        .with_releases(WILD_HORSES_2009, vec![rel(&honk()), rel(&sticky_fingers())])
        .with_releases(
            WILD_HORSES_SINGLE,
            vec![
                rel(&sticky_fingers()),
                rel(&group(
                    "6e33402c-62db-3dc9-af86-2c1bef99ce2f",
                    "Hot Rocks 1964–1971",
                    PrimaryType::Album,
                    &[SecondaryType::Compilation],
                    1971,
                    stones(),
                )),
            ],
        )
        .with_releases(WILD_HORSES_COMP, vec![rel(&honk())])
    }

    /// Wes Montgomery "Four on Six" on the live "Smokin' at the Half Note", credited to
    /// the Wynton Kelly Trio & Wes Montgomery: a distinct live recording.
    pub fn four_on_six(self) -> Self {
        let live_at_the_half_note = group(
            "44fa8c7f-a29e-4961-bf86-61bc62519288",
            "Live at the Half Note",
            PrimaryType::Album,
            &[SecondaryType::Live],
            2005,
            wynton_and_wes(),
        );
        self.with_search(
            "Four on Six",
            "Wes Montgomery",
            Some("Smokin' at the Half Note"),
            vec![
                hit(
                    FOUR_ON_SIX_LIVE,
                    100,
                    "Four on Six",
                    "",
                    405_426,
                    wynton_and_wes(),
                    vec![rel(&live_at_the_half_note), rel(&smokin())],
                ),
                hit(
                    "f267f4e7-1394-44f4-b310-e9dadb08b248",
                    85,
                    "Four on Six",
                    "",
                    569_000,
                    wynton_and_wes(),
                    vec![rel(&group(
                        "13f78733-63f7-4cd1-ae30-7d597bc436ee",
                        "Smokin' at the Half Note Vol. 2",
                        PrimaryType::Album,
                        &[SecondaryType::Live],
                        2005,
                        wynton_and_wes(),
                    ))],
                ),
            ],
        )
        .with_recording(detail(
            FOUR_ON_SIX_LIVE,
            "Four on Six",
            "",
            405_426,
            wynton_and_wes(),
            &[],
            vec![],
        ))
        .with_releases(
            FOUR_ON_SIX_LIVE,
            vec![rel(&live_at_the_half_note), rel(&smokin())],
        )
    }

    /// Bill Evans "Waltz for Debby", take 2 (the master take), credited "Bill Evans Trio
    /// with Scott LaFaro, Paul Motian". Its official releases include the 1962 live album
    /// and a same-titled 2012 compilation, which the browse lists first.
    pub fn waltz_for_debby(self) -> Self {
        self.with_recording(detail(
            WALTZ_FOR_DEBBY_TAKE_2,
            "Waltz for Debby",
            "take 2 - master",
            420_000,
            evans_trio_with_lafaro_and_motian(),
            &[],
            vec![],
        ))
        .with_releases(
            WALTZ_FOR_DEBBY_TAKE_2,
            vec![
                rel(&group(
                    "3706fa92-d486-414f-892a-7b21d0f6037f",
                    "Waltz for Debby",
                    PrimaryType::Album,
                    &[SecondaryType::Compilation],
                    2012,
                    bill_evans_trio(),
                )),
                rel(&waltz_for_debby_live()),
                rel(&group(
                    "c3007b4c-34e4-473c-ae13-294248eb3222",
                    "Seven Classic Albums",
                    PrimaryType::Album,
                    &[SecondaryType::Compilation],
                    2012,
                    bill_evans_trio(),
                )),
            ],
        )
    }
}

pub(crate) fn waltz_for_debby_live() -> ReleaseGroup {
    group(
        "392e7fcf-5ae9-37f6-b8c9-57901cfbee1f",
        "Waltz for Debby",
        PrimaryType::Album,
        &[SecondaryType::Live],
        1962,
        bill_evans_trio(),
    )
}

pub(crate) fn bill_evans_trio() -> ArtistCredit {
    credit(&[(
        "d0630a08-3b40-4cb4-9f48-7d525262c1f6",
        "Bill Evans Trio",
        "",
    )])
}

pub(crate) fn evans_trio_with_lafaro_and_motian() -> ArtistCredit {
    credit(&[
        (
            "d0630a08-3b40-4cb4-9f48-7d525262c1f6",
            "Bill Evans Trio",
            " with ",
        ),
        ("97cc902a-2850-4db8-8218-186f3b5a1ea5", "Scott LaFaro", ", "),
        ("9be59e25-f609-46b2-90cb-2db39b1a1910", "Paul Motian", ""),
    ])
}

pub(crate) const WALTZ_FOR_DEBBY_TAKE_2: &str = "d31d4528-14cf-48d9-960d-6a07eae19760";

pub(crate) const COCAINE_CREAM: &str = "aca688be-742b-4c44-96b9-2278bdc483c3";
pub(crate) const COCAINE_STUDIO: &str = "40ae4c06-3799-413b-b879-101dec39ac44";
pub(crate) const COCAINE_51: &str = "c2ee8d1d-0ae1-444b-8fb5-dbdaa7750cf5";
pub(crate) const COCAINE_WORK: &str = "a229efd2-a7fe-30b6-8f5b-1a5b86649690";
pub(crate) const WILD_HORSES_2009: &str = "c02a915c-a9e5-41f4-bf3a-ca3973723dd7";
pub(crate) const WILD_HORSES_SINGLE: &str = "1674d24d-8b74-4e65-94e5-e38527f593a9";
pub(crate) const WILD_HORSES_COMP: &str = "3afa1e36-63c9-4193-8807-c7b579607048";
pub(crate) const FOUR_ON_SIX_LIVE: &str = "1d97f7ab-2fc9-4cab-b3e3-bf6ec4d106ff";

pub(crate) fn slowhand() -> ReleaseGroup {
    group(
        "02c2b0c7-065d-38b4-8ac0-3391839f2418",
        "Slowhand",
        PrimaryType::Album,
        &[],
        1977,
        clapton(),
    )
}

pub(crate) fn slowhand_at_70() -> ReleaseGroup {
    group(
        "aa393ea3-05a2-45e0-87f7-cdc5664bc7b7",
        "Slowhand at 70: Live at the Royal Albert Hall",
        PrimaryType::Album,
        &[SecondaryType::Live],
        2015,
        clapton(),
    )
}

pub(crate) fn cream_of_clapton() -> ReleaseGroup {
    group(
        "976d6745-0091-39f1-ae60-c142986485a5",
        "The Cream of Clapton",
        PrimaryType::Album,
        &[SecondaryType::Compilation],
        1994,
        clapton(),
    )
}

pub(crate) fn sticky_fingers() -> ReleaseGroup {
    group(
        "4a4213d0-f224-3d51-bcca-476f61162681",
        "Sticky Fingers",
        PrimaryType::Album,
        &[],
        1971,
        stones(),
    )
}

pub(crate) fn honk() -> ReleaseGroup {
    group(
        "60b68a26-8799-4bd7-a46c-3bb67de14047",
        "Honk",
        PrimaryType::Album,
        &[SecondaryType::Compilation],
        2019,
        stones(),
    )
}

pub(crate) fn smokin() -> ReleaseGroup {
    group(
        "1f9c2a56-fdb7-38f1-95e2-ea4e7d946de8",
        "Smokin' at the Half Note",
        PrimaryType::Album,
        &[SecondaryType::Live],
        1965,
        wynton_and_wes(),
    )
}

pub(crate) fn clapton() -> ArtistCredit {
    credit(&[("618b6900-0618-4f1e-b835-bccb17f84294", "Eric Clapton", "")])
}

pub(crate) fn stones() -> ArtistCredit {
    credit(&[(
        "b071f9fa-14b0-4217-8e97-eb41da73f598",
        "The Rolling Stones",
        "",
    )])
}

pub(crate) fn wynton_and_wes() -> ArtistCredit {
    credit(&[
        (
            "8218b061-2d51-4375-affd-2e272083d12a",
            "Wynton Kelly Trio",
            " & ",
        ),
        ("663f8232-8c46-4851-803f-a91d31593b14", "Wes Montgomery", ""),
    ])
}

pub(crate) fn various_artists() -> ArtistCredit {
    credit(&[(
        "89ad4ac3-39f7-470e-963a-56509c546377",
        "Various Artists",
        "",
    )])
}

/// `(artist id, credited name, joinphrase)` per part.
pub(crate) fn credit(parts: &[(&str, &str, &str)]) -> ArtistCredit {
    ArtistCredit {
        parts: parts
            .iter()
            .map(|(id, name, join)| CreditPart {
                artist: super::mb::ArtistId((*id).into()),
                name: (*name).into(),
                joinphrase: (*join).into(),
            })
            .collect(),
    }
}

pub(crate) fn group(
    id: &str,
    title: &str,
    primary: PrimaryType,
    secondary: &[SecondaryType],
    year: u16,
    credit: ArtistCredit,
) -> ReleaseGroup {
    ReleaseGroup {
        id: ReleaseGroupId(id.into()),
        title: title.into(),
        primary: Some(primary),
        secondary: secondary.to_vec(),
        first_release: Some(PartialDate {
            year,
            month: None,
            day: None,
        }),
        credit,
    }
}

/// An official release titled like its group, dated with the group's first release.
pub(crate) fn rel(group: &ReleaseGroup) -> ReleaseRef {
    ReleaseRef {
        title: group.title.clone(),
        official: true,
        date: group.first_release,
        group: group.clone(),
    }
}

pub(crate) fn hit(
    id: &str,
    score: u8,
    title: &str,
    disambiguation: &str,
    length_ms: u32,
    credit: ArtistCredit,
    snippet_releases: Vec<ReleaseRef>,
) -> RecordingHit {
    RecordingHit {
        id: rid(id),
        score,
        title: title.into(),
        disambiguation: disambiguation.into(),
        length_ms: Some(length_ms),
        credit,
        snippet_releases,
    }
}

pub(crate) fn detail(
    id: &str,
    title: &str,
    disambiguation: &str,
    length_ms: u32,
    credit: ArtistCredit,
    isrcs: &[&str],
    works: Vec<WorkLink>,
) -> RecordingDetail {
    RecordingDetail {
        id: rid(id),
        title: title.into(),
        disambiguation: disambiguation.into(),
        length_ms: Some(length_ms),
        credit,
        isrcs: isrcs.iter().map(|i| Isrc((*i).into())).collect(),
        works,
    }
}

pub(crate) fn work(id: &str, attributes: &[&str]) -> WorkLink {
    WorkLink {
        work: WorkId(id.into()),
        attributes: attributes.iter().map(|a| (*a).into()).collect(),
    }
}

pub(crate) fn rid(id: &str) -> RecordingId {
    RecordingId(id.into())
}

pub(crate) fn query(title: &str, artist: &str, release: Option<&str>) -> RecordingQuery {
    RecordingQuery {
        title: title.into(),
        artist: artist.into(),
        release: release.map(str::to_string),
    }
}

#[async_trait::async_trait]
impl MbLookup for FixtureMb {
    async fn search_recordings(
        &self,
        query: &RecordingQuery,
    ) -> Result<(Vec<RecordingHit>, Truncated), MbError> {
        let release = query
            .release
            .as_deref()
            .map(|r| format!(" | {r}"))
            .unwrap_or_default();
        self.record(format!(
            "search {} | {}{release}",
            query.title, query.artist
        ))?;
        Ok((
            self.searches.get(query).cloned().unwrap_or_default(),
            Truncated(self.truncated_searches.contains(query)),
        ))
    }

    async fn recording(&self, id: &RecordingId) -> Result<RecordingDetail, MbError> {
        self.record(format!("recording {}", id.0))?;
        self.recordings.get(id).cloned().ok_or(MbError::NotFound)
    }

    async fn isrc_members(&self, isrc: &Isrc) -> Result<Vec<RecordingId>, MbError> {
        self.record(format!("isrc {}", isrc.0))?;
        self.isrcs.get(isrc).cloned().ok_or(MbError::NotFound)
    }

    async fn work_recordings(
        &self,
        work: &WorkId,
        cap: usize,
    ) -> Result<(Vec<RecordingId>, Truncated), MbError> {
        self.record(format!("work {}", work.0))?;
        let all = self.works.get(work).cloned().unwrap_or_default();
        let truncated = Truncated(all.len() > cap);
        Ok((all.into_iter().take(cap).collect(), truncated))
    }

    async fn recording_releases(
        &self,
        id: &RecordingId,
        cap: usize,
    ) -> Result<(Vec<ReleaseRef>, Truncated), MbError> {
        self.record(format!("releases {}", id.0))?;
        let all = self.releases.get(id).cloned().unwrap_or_default();
        let truncated = Truncated(all.len() > cap);
        Ok((all.into_iter().take(cap).collect(), truncated))
    }
}

impl super::lane::LaneMb for FixtureMb {
    fn counters(&self) -> (u64, u64) {
        (self.calls.lock().unwrap().len() as u64, 0)
    }
}

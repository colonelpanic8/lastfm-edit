use super::*;
use http_client::http_types::Response;
use std::collections::VecDeque;
use std::sync::Mutex;

const SEARCH: &str = include_str!("fixtures/search_cocaine_cream_of_clapton.json");
const RECORDING_CREAM: &str = include_str!("fixtures/recording_aca688be.json");
const RECORDING_SLOWHAND: &str = include_str!("fixtures/recording_40ae4c06.json");
const ISRC: &str = include_str!("fixtures/isrc_NLF057790024.json");
const RELEASES: &str = include_str!("fixtures/release_browse_40ae4c06.json");
const WORK: &str = include_str!("fixtures/work_browse_a229efd2.json");

const CREAM_COCAINE: &str = "aca688be-742b-4c44-96b9-2278bdc483c3";
const SLOWHAND_COCAINE: &str = "40ae4c06-3799-413b-b879-101dec39ac44";
const COCAINE_WORK: &str = "a229efd2-a7fe-30b6-8f5b-1a5b86649690";
const CLAPTON: &str = "618b6900-0618-4f1e-b835-bccb17f84294";
const USER_AGENT: &str = "scrobble-scrubber-dev/0.2 ( ivanmalison@gmail.com )";

type Route = Box<dyn Fn(&Url) -> Response + Send + Sync>;

struct FakeMb {
    route: Route,
    seen: Mutex<Vec<(String, String, String)>>,
}

impl std::fmt::Debug for FakeMb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FakeMb")
    }
}

#[http_client::async_trait]
impl http_client::HttpClient for FakeMb {
    async fn send(&self, request: Request) -> http_client::http_types::Result<Response> {
        let header = |name| {
            request
                .header(name)
                .map(|values| values.last().as_str().to_string())
                .unwrap_or_default()
        };
        self.seen.lock().unwrap().push((
            request.url().to_string(),
            header("User-Agent"),
            header("Accept"),
        ));
        Ok((self.route)(request.url()))
    }
}

impl FakeMb {
    fn new(route: impl Fn(&Url) -> Response + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self {
            route: Box::new(route),
            seen: Mutex::new(Vec::new()),
        })
    }

    fn urls(&self) -> Vec<String> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .map(|(url, ..)| url.clone())
            .collect()
    }
}

fn client(http: &Arc<FakeMb>) -> (MusicBrainz, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let user_agent = UserAgent::new("scrobble-scrubber-dev/0.2", "ivanmalison@gmail.com").unwrap();
    let mb = MusicBrainz::new(
        http.clone(),
        user_agent,
        &dir.path().join("cache/musicbrainz.sqlite"),
    )
    .unwrap();
    (mb, dir)
}

fn ok(body: impl Into<String>) -> Response {
    let mut response = Response::new(StatusCode::Ok);
    response.set_body(body.into());
    response
}

fn status(status: StatusCode, headers: &[(&str, &str)]) -> Response {
    let mut response = Response::new(status);
    for (name, value) in headers {
        response.insert_header(*name, *value).unwrap();
    }
    response
}

fn param(url: &Url, name: &str) -> Option<String> {
    url.query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

fn clapton() -> ArtistCredit {
    ArtistCredit {
        parts: vec![CreditPart {
            artist: ArtistId(CLAPTON.to_string()),
            name: "Eric Clapton".to_string(),
            joinphrase: String::new(),
        }],
    }
}

fn date(year: u16, month: Option<u8>, day: Option<u8>) -> Option<PartialDate> {
    Some(PartialDate { year, month, day })
}

/// Release browse pages of at most 4 rows, the way MB truncates pages below `limit`.
fn release_pages(reported_count: usize) -> impl Fn(&Url) -> Response {
    move |url| {
        let mut page: serde_json::Value = serde_json::from_str(RELEASES).unwrap();
        let offset: usize = param(url, "offset").unwrap().parse().unwrap();
        let rows = page["releases"].as_array().unwrap().clone();
        let end = rows.len().min(offset + 4);
        page["releases"] = serde_json::Value::from(rows[offset.min(end)..end].to_vec());
        page["release-count"] = reported_count.into();
        page["release-offset"] = offset.into();
        ok(page.to_string())
    }
}

#[tokio::test(start_paused = true)]
async fn search_parses_hits_and_repeats_from_cache() {
    let http = FakeMb::new(|_| ok(SEARCH));
    let (mb, _dir) = client(&http);
    let query = RecordingQuery {
        title: "Cocaine".to_string(),
        artist: "Eric Clapton".to_string(),
        release: Some("The Cream of Clapton".to_string()),
    };

    let (hits, truncated) = mb.search_recordings(&query).await.unwrap();

    assert_eq!(truncated, Truncated(false));
    assert_eq!(
        hits.iter()
            .map(|h| (
                h.id.0.as_str(),
                h.score,
                h.title.as_str(),
                h.disambiguation.as_str(),
                h.length_ms,
                h.credit.display()
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                CREAM_COCAINE,
                100,
                "Cocaine",
                "",
                Some(218747),
                "Eric Clapton".to_string()
            ),
            (
                SLOWHAND_COCAINE,
                56,
                "Cocaine",
                "original studio mix",
                Some(216640),
                "Eric Clapton".to_string()
            ),
        ]
    );
    assert_eq!(
        hits[0].snippet_releases,
        vec![ReleaseRef {
            title: "The Cream of Clapton".to_string(),
            official: true,
            date: None,
            group: ReleaseGroup {
                id: ReleaseGroupId("976d6745-0091-39f1-ae60-c142986485a5".to_string()),
                title: "The Cream of Clapton".to_string(),
                primary: Some(PrimaryType::Album),
                secondary: vec![SecondaryType::Compilation],
                first_release: None,
                credit: clapton(),
            },
        }]
    );
    assert_eq!(
        hits[1]
            .snippet_releases
            .iter()
            .map(|r| (
                r.title.as_str(),
                r.official,
                r.date,
                r.group.secondary.clone(),
                r.group.credit.is_various_artists()
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                "Favourite Rock Artist Series",
                false,
                None,
                vec![SecondaryType::Compilation],
                false
            ),
            ("Slowhand", true, None, vec![], false),
            (
                "Soft Metal Ballads",
                true,
                date(1991, None, None),
                vec![SecondaryType::Compilation],
                true
            ),
            (
                "The Cream of Clapton",
                true,
                None,
                vec![SecondaryType::Compilation],
                false
            ),
        ]
    );

    assert_eq!(mb.search_recordings(&query).await.unwrap().0, hits);
    assert_eq!(
            http.urls(),
            vec!["https://musicbrainz.org/ws/2/recording?query=recording%3A%22Cocaine%22+AND+artist%3A%22Eric+Clapton%22+AND+release%3A%22The+Cream+of+Clapton%22&limit=100&fmt=json"]
        );
    let (_, user_agent, accept) = http.seen.lock().unwrap()[0].clone();
    assert_eq!(
        (user_agent.as_str(), accept.as_str()),
        (USER_AGENT, "application/json")
    );
    assert_eq!(
        mb.stats(),
        MbStats {
            requests: 1,
            cache_hits: 1
        }
    );
}

#[tokio::test(start_paused = true)]
async fn search_release_without_credits_is_credited_like_the_recording() {
    let mut search: serde_json::Value = serde_json::from_str(SEARCH).unwrap();
    search["recordings"][0]["releases"][0]
        .as_object_mut()
        .unwrap()
        .remove("artist-credit");
    let http = FakeMb::new(move |_| ok(search.to_string()));
    let (mb, _dir) = client(&http);

    let (hits, _) = mb
        .search_recordings(&RecordingQuery {
            title: "Cocaine".to_string(),
            artist: "Eric Clapton".to_string(),
            release: Some("The Cream of Clapton".to_string()),
        })
        .await
        .unwrap();

    assert_eq!(hits[0].snippet_releases[0].group.credit, clapton());
}

#[tokio::test(start_paused = true)]
async fn search_reports_matches_beyond_the_returned_page() {
    let query = RecordingQuery {
        title: "Cocaine".to_string(),
        artist: "Eric Clapton".to_string(),
        release: None,
    };
    for (count, truncated) in [(2, false), (348, true)] {
        let mut search: serde_json::Value = serde_json::from_str(SEARCH).unwrap();
        search["count"] = count.into();
        let http = FakeMb::new(move |_| ok(search.to_string()));
        let (mb, _dir) = client(&http);

        let (hits, got) = mb.search_recordings(&query).await.unwrap();

        assert_eq!(
            (hits.len(), got),
            (2, Truncated(truncated)),
            "count {count}"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn searches_cached_without_a_truncation_flag_are_fetched_again() {
    let http = FakeMb::new(|_| ok(SEARCH));
    let (mb, _dir) = client(&http);
    let query = RecordingQuery {
        title: "Cocaine".to_string(),
        artist: "Eric Clapton".to_string(),
        release: Some("The Cream of Clapton".to_string()),
    };
    let stale: Vec<RecordingHit> = Vec::new();
    mb.cache
        .put(&format!("search:{}", query.lucene()), &stale)
        .unwrap();

    let (hits, truncated) = mb.search_recordings(&query).await.unwrap();

    assert_eq!((hits.len(), truncated), (2, Truncated(false)));
    assert_eq!(http.urls().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn recording_and_isrc_lookups_carry_identity_evidence() {
    let http = FakeMb::new(|url| match url.path() {
        "/ws/2/isrc/NLF057790024" => ok(ISRC),
        _ => ok(RECORDING_CREAM),
    });
    let (mb, _dir) = client(&http);

    let detail = mb
        .recording(&RecordingId(CREAM_COCAINE.to_string()))
        .await
        .unwrap();
    let members = mb
        .isrc_members(&Isrc("NLF057790024".to_string()))
        .await
        .unwrap();

    assert_eq!(
        detail,
        RecordingDetail {
            id: RecordingId(CREAM_COCAINE.to_string()),
            title: "Cocaine".to_string(),
            disambiguation: String::new(),
            length_ms: Some(218747),
            credit: clapton(),
            isrcs: vec![Isrc("NLF057790024".to_string())],
            works: vec![WorkLink {
                work: WorkId(COCAINE_WORK.to_string()),
                attributes: vec![],
            }],
        }
    );
    assert_eq!(
        members,
        vec![
            RecordingId(CREAM_COCAINE.to_string()),
            RecordingId(SLOWHAND_COCAINE.to_string()),
        ]
    );
    assert_eq!(
            http.urls(),
            vec![
                format!("https://musicbrainz.org/ws/2/recording/{CREAM_COCAINE}?inc=isrcs+work-rels+artist-credits&fmt=json"),
                "https://musicbrainz.org/ws/2/isrc/NLF057790024?fmt=json".to_string(),
            ]
        );
}

#[tokio::test(start_paused = true)]
async fn merged_recording_follows_redirect_and_caches_under_requested_id() {
    const MERGED: &str = "5e0d1c32-0000-4000-8000-00000000c0ca";
    let http = FakeMb::new(|url| {
        if url.path().ends_with(MERGED) {
            let location = format!("https://musicbrainz.org/ws/2/recording/{SLOWHAND_COCAINE}?inc=isrcs+work-rels+artist-credits&fmt=json");
            status(StatusCode::MovedPermanently, &[("Location", &location)])
        } else {
            ok(RECORDING_SLOWHAND)
        }
    });
    let (mb, _dir) = client(&http);
    let requested = RecordingId(MERGED.to_string());

    let detail = mb.recording(&requested).await.unwrap();

    assert_eq!(detail.id, RecordingId(SLOWHAND_COCAINE.to_string()));
    assert_eq!(detail.disambiguation, "original studio mix");
    assert_eq!(
        detail.isrcs,
        ["GBA077700020", "NLF057790024", "USRY20517111"].map(|code| Isrc(code.to_string()))
    );
    assert_eq!(
        detail.works,
        vec![WorkLink {
            work: WorkId(COCAINE_WORK.to_string()),
            attributes: vec!["cover".to_string()],
        }]
    );
    assert_eq!(mb.recording(&requested).await.unwrap(), detail);
    assert_eq!(http.urls().len(), 2);
    assert_eq!(
        mb.stats(),
        MbStats {
            requests: 2,
            cache_hits: 1
        }
    );
}

#[tokio::test(start_paused = true)]
async fn rate_limits_report_the_wait_and_hold_back_the_next_request() {
    let unavailable = || status(StatusCode::ServiceUnavailable, &[]);
    let responses = Mutex::new(VecDeque::from([
        status(StatusCode::ServiceUnavailable, &[("Retry-After", "7")]),
        status(StatusCode::ServiceUnavailable, &[("Retry-After", "0")]),
        status(StatusCode::TooManyRequests, &[]),
        unavailable(),
        unavailable(),
        unavailable(),
        unavailable(),
        ok(ISRC),
        unavailable(),
    ]));
    let calls = responses.lock().unwrap().len();
    let http = FakeMb::new(move |_| responses.lock().unwrap().pop_front().unwrap());
    let (mb, _dir) = client(&http);

    let mut outcomes = Vec::new();
    for call in 0..calls {
        let start = Instant::now();
        let result = mb.isrc_members(&Isrc(format!("TEST{call}"))).await;
        let waited = start.elapsed().as_secs();
        outcomes.push(match result {
            Ok(_) => (waited, None),
            Err(MbError::RateLimited(wait)) => (waited, Some(wait.as_secs())),
            Err(other) => panic!("unexpected {other:?}"),
        });
    }

    assert_eq!(
        outcomes,
        [
            (0, Some(7)),
            (7, Some(2)),
            (2, Some(4)),
            (4, Some(8)),
            (8, Some(16)),
            (16, Some(32)),
            (32, Some(60)),
            (60, None),
            (1, Some(2)),
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn uncached_requests_are_spaced_and_cache_hits_skip_the_pacer() {
    let http = FakeMb::new(|_| ok(ISRC));
    let (mb, _dir) = client(&http);

    let start = Instant::now();
    mb.isrc_members(&Isrc("NLF057790024".to_string()))
        .await
        .unwrap();
    mb.isrc_members(&Isrc("GBA077700020".to_string()))
        .await
        .unwrap();
    let paced = start.elapsed();
    mb.isrc_members(&Isrc("NLF057790024".to_string()))
        .await
        .unwrap();

    assert_eq!(paced.as_millis() / 100, 11);
    assert_eq!(start.elapsed(), paced);
    assert_eq!(
        mb.stats(),
        MbStats {
            requests: 2,
            cache_hits: 1
        }
    );
}

#[tokio::test(start_paused = true)]
async fn release_browse_parses_release_groups() {
    let http = FakeMb::new(release_pages(6));
    let (mb, _dir) = client(&http);

    let (releases, truncated) = mb
        .recording_releases(&RecordingId(SLOWHAND_COCAINE.to_string()), 100)
        .await
        .unwrap();

    use PrimaryType::Album;
    use SecondaryType::{Compilation, Soundtrack};
    assert_eq!(truncated, Truncated(false));
    assert_eq!(
        releases
            .iter()
            .map(|r| (
                r.title.as_str(),
                r.official,
                r.date,
                r.group.primary,
                r.group.secondary.clone(),
                r.group.first_release,
                r.group.credit.display()
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                "The Cream of Eric Clapton",
                true,
                date(1987, Some(9), None),
                Some(Album),
                vec![Compilation],
                date(1987, Some(9), None),
                "Eric Clapton".to_string()
            ),
            (
                "Slowhand",
                true,
                date(1977, None, None),
                Some(Album),
                vec![],
                date(1977, None, None),
                "Eric Clapton".to_string()
            ),
            (
                "The Cream of Clapton",
                true,
                date(1994, None, None),
                Some(Album),
                vec![Compilation],
                date(1994, Some(3), Some(29)),
                "Eric Clapton".to_string()
            ),
            (
                "Bad News Bears",
                true,
                date(2005, Some(7), Some(26)),
                Some(Album),
                vec![Soundtrack],
                date(2005, Some(1), Some(1)),
                "Various Artists".to_string()
            ),
            (
                "Crossroads",
                true,
                date(1996, Some(11), Some(4)),
                Some(Album),
                vec![Compilation],
                date(1988, Some(4), Some(18)),
                "Eric Clapton".to_string()
            ),
            (
                "Stereoplay: The Best of Highlights II",
                true,
                date(1986, None, None),
                Some(Album),
                vec![Compilation],
                date(1986, None, None),
                "Various Artists".to_string()
            ),
        ]
    );
    assert_eq!(
        releases[1].group.id,
        ReleaseGroupId("02c2b0c7-065d-38b4-8ac0-3391839f2418".to_string())
    );
    assert_eq!(
            http.urls()[0],
            format!("https://musicbrainz.org/ws/2/release?recording={SLOWHAND_COCAINE}&status=official&inc=release-groups+artist-credits&limit=100&offset=0&fmt=json")
        );
}

#[tokio::test(start_paused = true)]
async fn release_paging_advances_by_rows_returned_and_stops_at_the_cap() {
    let cases: [(usize, usize, &[&str], usize, bool); 4] = [
        (6, 100, &["0", "4"], 6, false),
        (6, 6, &["0", "4"], 6, false),
        (168, 5, &["0", "4"], 5, true),
        (168, 4, &["0"], 4, true),
    ];
    for (count, cap, offsets, kept, truncated) in cases {
        let http = FakeMb::new(release_pages(count));
        let (mb, _dir) = client(&http);
        let id = RecordingId(SLOWHAND_COCAINE.to_string());

        let (releases, got_truncated) = mb.recording_releases(&id, cap).await.unwrap();
        let requested: Vec<String> = http
            .urls()
            .iter()
            .map(|url| param(&Url::parse(url).unwrap(), "offset").unwrap())
            .collect();

        assert_eq!(requested, offsets, "count {count}, cap {cap}");
        assert_eq!(releases.len(), kept, "count {count}, cap {cap}");
        assert_eq!(
            got_truncated,
            Truncated(truncated),
            "count {count}, cap {cap}"
        );

        assert_eq!(mb.recording_releases(&id, cap).await.unwrap().0, releases);
        assert_eq!(
            http.urls().len(),
            offsets.len(),
            "repeat is served from cache"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn work_browse_reports_truncation_at_the_cap() {
    let http = FakeMb::new(|_| ok(WORK));
    let (mb, _dir) = client(&http);

    let (recordings, truncated) = mb
        .work_recordings(&WorkId(COCAINE_WORK.to_string()), 3)
        .await
        .unwrap();

    assert_eq!(
        recordings,
        [
            "04cfc359-e11a-46a1-ab11-0be97b2a3e3d",
            "069e43a4-6a14-46d6-9d86-ba54fa5a640c",
            "07a57746-d956-4053-9b54-39ef8e6d49ff",
        ]
        .map(|id| RecordingId(id.to_string()))
    );
    assert_eq!(truncated, Truncated(true));
    assert_eq!(
            http.urls(),
            vec![format!("https://musicbrainz.org/ws/2/recording?work={COCAINE_WORK}&limit=100&offset=0&fmt=json")]
        );
}

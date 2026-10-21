# Changelog

## Unreleased

## lastfm-edit 8.0.0 (2026-10-21)

- Return structured track-page failures with the track, artist, URL, and a typed
  `ScrobblePageError` source (`LoggedOut`, `MissingChartlist`, or `MissingEditForms`).
  Callers of `get_scrobble_edit_variations` should match `LastFmError::ScrobblePage`
  instead of inspecting `Auth` or `Parse` strings for these page-content failures.
  HTTP authentication failures can still return `LastFmError::Auth`.
- Remove the automatic HTML dump to a fixed `/tmp` path and the duplicate warning.
- Emit scrobble-discovery diagnostics with structured key/value fields.
- Treat session expiry during pagination as an error instead of returning partial results.

## Scrobble Scrubber 0.2.0 (2026-10-18)

- Replace queued human decisions with immediate async handle methods. Decisions return
  their result directly and rejection interrupts active execution.
- Hold a per-store execution lease so simultaneous app and CLI runs cannot submit the
  same edit twice.
- Keep the partition lock across compaction reads and replacement, preserving scrobbles
  and tombstones appended concurrently by another process.
- Show Last.fm album covers and artist photos in the library and intent views.

## Scrobble Scrubber 0.1.5 (2026-10-17)

- The library's all-time view and artist drill-downs read counts straight from covering
  indexes instead of decoding every stored record: on a 191k-scrobble store the default
  library view drops from about 295 ms to 45 ms and an artist view from 115 ms to 6 ms.
- Rebuilding the SQLite index loads every month in one transaction and creates indexes
  afterwards, cutting a full rebuild from 10–51 s to about 2 s. The index schema version
  increases, so the first launch after updating rebuilds the index once.
- Rule matching no longer formats a diagnostic for every rule that fails to match,
  roughly halving the CPU time of a full-history replan.

## lastfm-edit 7.0.2 (2026-10-16)

- Remove passwords, cookies, tokens, and response bodies from login diagnostics.
- Reject authenticated requests and redirects outside the configured session origin,
  including HTTPS downgrades.
- Preserve authentication, cancellation, transport, and rate-limit errors through exact
  edits and deletes so callers can choose the appropriate recovery policy.
- Retain cancellation requested before a receiver subscribes, and keep dropped senders
  from spuriously cancelling waits.

## Scrobble Scrubber 0.1.4 (2026-10-16)

- Incremental sync no longer loses scrobbles. Last.fm stamps a scrobble with the track's
  start time but records it only after the track finishes, so syncs that ran mid-track
  marked that time covered before the scrobble existed. With five-minute polling about
  half of all scrobbles were never stored. Each sync now reconciles the trailing 24 hours
  (`SyncOptions::late_arrival_secs`) against upstream, adding late arrivals and removing
  records that are gone.
- Recently mirrored edits converge to what last.fm actually stored. Last.fm keeps its own
  casing for existing tracks, so an edit to "Not A Second Time" is stored as "Not a Second
  Time", and the store kept the requested spelling as a phantom duplicate.
- Re-fetching an unchanged scrobble no longer appends a duplicate line, and an API
  re-fetch keeps a known album artist instead of downgrading it to unknown.
- Incremental planning durably tracks analyzed record revisions, finding late arrivals
  and metadata-only updates even after restart or lost discovery notifications.
- Planning claims only synced intervals. Explicit whole-store passes force a rescan,
  and each pass uses one current rule snapshot for both analysis and coverage hashing.
- Rejecting an intent interrupts waits and prevents remaining edits; late executor events
  preserve the rejected state. Authentication failures leave work available for recovery.
- Stable per-instance operation IDs reconcile interrupted upstream edits before retrying,
  including paginated windows and album-artist-only edits. Ambiguous outcomes remain
  pending instead of being blindly submitted again.
- JSONL appends repair incomplete crash tails under writer serialization and report
  corruption in complete lines. Atomic snapshots sync their parent directories.
- The SQLite index detects same-size and growing rewrites of already-indexed bytes.
- Continuous runs await actual cycle completion and invalidate queued work when disabled,
  while manual commands remain independent. The app and CLI share the same cycle path.

## Scrobble Scrubber 0.1.3 (2026-08-12)

- The daemon validates the saved last.fm session at startup and re-logs-in automatically
  using the new `[lastfm] password` config key (or `LASTFM_EDIT_PASSWORD`); without
  credentials it refuses to start instead of running every edit into failures against
  logged-out pages. Last.fm sessions expire after about a year, which previously left the
  daemon syncing and planning for weeks while no edit could succeed.
- `run` mode exits non-zero when the executor task dies instead of continuing to sync and
  plan as a zombie. A panic in the HTTP stack (see lastfm-edit 7.0.1 below) killed the
  executor task silently on every daemon start for ten days while cycles kept printing.
- The executor aborts a pass on authentication failures instead of counting them against
  each intent's attempt budget — an expired session no longer abandons intents that would
  succeed after re-login.

## Scrobble Scrubber 0.1.2 (2026-07-13)

- Add an Android build with a mobile-responsive interface and app-private storage.
- Make the existing Last.fm username/password form work on fresh mobile installs without
  relying on desktop environment variables or `pass`; passwords are never persisted.
- Use a cross-platform Rustls HTTP transport that preserves the login response semantics
  needed to capture Last.fm session cookies.
- Add tag-driven release automation for a signed Android APK/AAB and Linux desktop
  package. The GitHub release is published only after both builds pass.

## lastfm-edit 7.0.1 (2026-08-12)

### Breaking changes

- **API-sourced tracks no longer fabricate `Track.album_artist`.** The official
  recent-tracks API provides no album-artist field; the parser previously copied the track
  artist into it, which is wrong for compilations and soundtracks. API-sourced tracks now
  carry `album_artist: None`. Scraped edit-form values remain the authoritative source.
- **`ScrobbleEdit` constructors stopped guessing album artists.** `from_track_info`,
  `with_minimal_info`, and `from_track_and_artist` now pass `None` for
  `album_artist_name_original` (discovery no longer filters on a guess) *and* for the new
  `album_artist_name` (the edit keeps whatever the form already has, instead of silently
  rewriting a compilation's album artist to the track artist).
- **`ClientConfig` gained a `rate_limit_behavior` field** (see below); code constructing it
  field-by-field must add it. Constructors using `..Default::default()` are unaffected.
- **`LastFmApiClient` gained a required method** `api_get_recent_tracks_page_in_range`;
  the old `api_get_recent_tracks_page` is now a default method delegating to it.
- **`LastFmEditClient` gained a required method** `get_scrobble_edit_variations`
  (the former `load_edit_form_values_internal`, now public and properly named; the old
  name remains as a deprecated alias on `LastFmEditClientImpl`).

### New features

- **Queryable rate-limit state.** `RateLimitState` (`Ready` /
  `RateLimited { since, until_estimate, kind }`) is derived automatically from the event
  stream and exposed via `rate_limit_state()` / `watch_rate_limit_state()` on
  `SharedEventBroadcaster`, `LastFmEditClientImpl`, `LastFmApiClientImpl`, and (with a
  default implementation) `LastFmBaseClient`. Await `.changed()` on the watcher to react
  to pause/resume without polling.
- **Non-blocking rate-limit mode.** `RateLimitBehavior::ReturnError` (opt in via
  `ClientConfig` or `client.non_blocking()`, which returns a config-flipped clone sharing
  the session and broadcaster) makes operations return
  `Err(LastFmError::RateLimit { retry_after })` instead of sleeping inside retry loops —
  including `edit_scrobble_single` and `delete_scrobble`, which previously swallowed rate
  limits into `success: false` / `Ok(false)`. Callers own scheduling and can show
  "paused until T" UIs.
- **Time-windowed API fetching.** `api_get_recent_tracks_page_in_range(page, from, to)`
  and `recent_tracks_in_range(from, to)` pass unix-timestamp windows to
  `user.getRecentTracks`. Verified against the live service: the window is natively
  half-open — `from` inclusive, `to` exclusive (see the `api_recent_tracks_in_range` VCR
  test).
- **Album-artist backfill API.** `get_scrobble_edit_variations(track, artist)` returns the
  fully-populated `ExactScrobbleEdit` variations scraped from the edit forms (the
  authoritative album-artist source), and `resolve_album_artist(artist, track, album)` is
  a convenience wrapper over it.

### Fixes

- Expired or logged-out sessions surface as `LastFmError::Auth` when a track page renders
  without edit forms but with login links, instead of the misleading
  `Parse("No scrobble forms found…")` that was indistinguishable from a page-shape change.
- Panics inside the underlying HTTP client's `send` are caught and returned as request
  errors instead of unwinding through — and silently killing — caller tasks. (http-client's
  isahc adapter panics on server status codes outside http-types' `StatusCode` enum.)
- Pattern-detected rate limits now broadcast `ClientEvent::RateLimited` even when retries
  are disabled (previously the error surfaced with no event).
- The recent-tracks API parser accepts the API's single-track-as-object and empty-page
  response shapes, and surfaces API error bodies (`{"error":..,"message":..}`) as readable
  errors instead of parse failures.
- The duplicated recent-tracks request implementation in `LastFmEditClientImpl` was
  consolidated with `LastFmApiClientImpl` into one shared helper.

## scrobble-store 0.1.0 (new)

Initial release of the synchronizable local mirror of a Last.fm scrobble history:
append-only JSONL flat files in a git-friendly layout as the source of truth (LWW dedup by
scrobble id, `merge=union` friendly), a disposable SQLite query index, a coverage-segment
sync engine (deterministic `to`-pinned windows, per-page persistence, resumable backfill,
rate-limit-aware pause/resume, typed progress events), a durable mirrored-edit log with
crash-safe resume, and a CLI (`init`/`sync`/`backfill`/`status`/`coverage`/`verify`/
`invalidate`/`log`/`compact`/`reindex`/`query`) with optional git auto-commit.

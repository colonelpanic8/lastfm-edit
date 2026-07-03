# Changelog

## lastfm-edit 7.0.0 (unreleased)

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

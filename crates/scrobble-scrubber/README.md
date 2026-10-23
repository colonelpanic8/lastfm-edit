# scrobble-scrubber

Rule-driven cleanup of last.fm scrobble metadata, built on the
[`scrobble-store`](../scrobble-store) local mirror.

## Architecture

```
SyncEngine ─▶ scrobble-store ─▶ PLANNER ─▶ edit-intent queue ─▶ EXECUTOR ─▶ MirroredEditor ─▶ last.fm + store
```

Three loosely-coupled stages over durable state:

- **Planner** — conceives edits without ever touching last.fm. Resolves a *feed* (whole
  store, incremental over newly-synced scrobbles, one artist/album, explicit ids), dedupes
  scrobbles into metadata *subjects*, runs the provider stack (regex rewrite rules, and —
  behind features — MusicBrainz verification and an LLM suggester), and records durable
  *edit intents*. Per-provider planning coverage (a `CoverageMap` over scrobble time) makes
  incremental runs cheap and automatically picks up backfilled history; the rules
  provider's coverage resets itself whenever the rule set changes.
- **Edit-intent queue** — one event-sourced JSONL state machine
  (`AwaitingApproval → Ready → InProgress → Applied` / `Rejected` / `Abandoned`) that
  unifies approval workflow and execution backlog. Intents are subject-level and expanded
  to concrete scrobbles at *execution* time, so instances discovered after planning are
  included.
- **Executor** — the single paced lane owning ALL last.fm write traffic (edit posts *and*
  album-artist enrichment scrapes). Drains ready intents through the store's crash-safe
  `MirroredEditor`; rate limits pause it (never counted as failures), an inter-edit delay
  paces even success, and per-instance progress makes any interruption resumable.

Scrubber state lives in `<store_root>/scrubber/` as append-only JSONL + atomic JSON
snapshots (git `merge=union` friendly, like the store itself).

## CLI

```
scrobble-scrubber rules enable-defaults          # seed the ~74-rule cleanup corpus
scrobble-scrubber plan store --dry-run           # preview suggestions over the whole store
scrobble-scrubber plan incremental               # queue intents for newly-synced scrobbles
scrobble-scrubber queue list [--state ready]     # inspect; approve/reject awaiting items
scrobble-scrubber execute --max-edits 20         # bounded, paced application to last.fm
scrobble-scrubber run --interval 300             # continuous: sync + plan + execute
scrobble-scrubber coverage show|reset            # planning-coverage management
```

Credentials: sync uses `LASTFM_EDIT_USERNAME`/`LASTFM_EDIT_API_KEY`; editing uses a saved
lastfm-edit session (log in once with the `lastfm-edit` CLI). Optional config at
`~/.config/scrobble-scrubber/config.toml`:

```toml
[scrubber]
interval = 300
dry_run = false
require_confirmation = false
batch_size = 50

[store]
root = "/path/to/scrobble-store"   # default: ~/.local/share/scrobble-store/<username>
# state_dir = ""                   # default: <root>/scrubber

[executor]
inter_edit_delay_secs = 2            # sleep between edits; fractional values allowed
max_attempts_per_instance = 3
max_rate_limit_pauses_per_pass = 3   # rate-limit waits a pass sits through before deferring
rate_limit_pause_secs = 60           # first wait after a rate limit; doubles per consecutive block
max_rate_limit_pause_secs = 600
reuse_csrf_token = false             # skip loading the edit form before every edit
```

## Features

- `cli` (default) — the binary.
- `musicbrainz` — MusicBrainz verification gating (per-rule
  `requires_musicbrainz_confirmation` + release filters) and the compilation→canonical
  provider. Without it, rules wanting MusicBrainz confirmation degrade to *human*
  confirmation.
- `openai` — LLM provider that suggests one-off edits and proposes new rewrite rules
  (reviewed via `pending-rules`).

## Lineage

The rules engine (`SdRule`/`RewriteRule`, the default rule corpus, and its test suite) is
ported from the original [scrobble-scrubber](https://github.com/colonelpanic8/scrobble-scrubber)
nearly verbatim — same matching semantics, same serde format — with compiled-regex caching
and honest album-artist handling (never fabricated from the track artist). The driver is a
rewrite: the old anchor-timestamp/track-cache machinery is replaced by the scrobble-store
foundation.

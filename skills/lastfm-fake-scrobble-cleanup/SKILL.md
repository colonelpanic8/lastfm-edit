---
name: lastfm-fake-scrobble-cleanup
description: Identify and remove accidental or "fake" Last.fm scrobbles with the repo CLI. Use when a user says Spotify kept playing, a queue ran unattended, or a recent block of scrobbles should be deleted while preserving known-good plays.
---

# Lastfm Fake Scrobble Cleanup

## Overview

Use the repo CLI to find the contiguous block of unwanted recent scrobbles, preserve known-good anchor plays, preview the delete range, and only then execute the deletion.

Assume the repo command should be run via `nix develop -c cargo run -- ...` unless the environment is already prepared.

## Workflow

1. Inspect recent offsets.
2. Find known-good anchor scrobbles the user wants to keep.
3. Choose the delete boundary around those anchors.
4. Preview the exact delete range in dry-run mode.
5. Apply the deletion with explicit confirmation.
6. Report the final summary and the preserved offsets.

## Inspect Recent Offsets

Start with `show`, not `delete`.

Use a sparse sample first:

```bash
nix develop -c cargo run -- -q show 0 1 2 3 4 5 10 20 30 40 50 60 70 80 90
```

If the boundary is unclear, inspect a denser slice:

```bash
nix develop -c bash -lc 'cargo run -q -- -q show $(seq 0 140) 2>/dev/null | jq -r "select(.type==\"ScrobbleDetails\") | [.offset, .scrobble.timestamp, .scrobble.artist, .scrobble.name] | @tsv"'
```

Treat offset `0` as the most recent scrobble.

## Preserve Anchor Plays

Ask the user which recent plays are definitely real. Common examples:

- "I really listened to `No One Knows`."
- "Keep `Don't Let Me Down`."

Locate anchors by name before choosing the delete range:

```bash
nix develop -c bash -lc 'cargo run -q -- -q show $(seq 0 250) 2>/dev/null | jq -r "select(.type==\"ScrobbleDetails\") | [.offset, .scrobble.timestamp, .scrobble.artist, .scrobble.name] | @tsv" | rg -i "no one knows|don.t let me down"'
```

Do not recommend a delete range that includes a confirmed anchor play.

## Choose The Boundary

Prefer a contiguous `--recent-offset start-end` range.

Use one of these heuristics:

- Keep offset `0` if it is a real anchor, then start deletion at `1`.
- Stop deletion immediately before the next known-good anchor.
- If the boundary is still unclear, look for a large timestamp gap between adjacent offsets.

Useful gap scan:

```bash
nix develop -c bash -lc 'cargo run -q -- -q show $(seq 0 250) 2>/dev/null | jq -r "select(.type==\"ScrobbleDetails\") | [.offset, .scrobble.timestamp, .scrobble.artist, .scrobble.name] | @tsv" | awk -F"\t" "NR==1{prev_off=\$1; prev_ts=\$2; prev_artist=\$3; prev_track=\$4; next} {gap=prev_ts-\$2; if (gap >= 1200) printf \"gap=%5ds (%6.1f min) between offset %s [%s - %s] and %s [%s - %s]\\n\", gap, gap/60, prev_off, prev_artist, prev_track, \$1, \$3, \$4; prev_off=\$1; prev_ts=\$2; prev_artist=\$3; prev_track=\$4 }"'
```

Summarize the proposed range in plain English before applying it.

Example:

- "Offsets `1-89` look like the fake block."
- "Offset `0` is a real `No One Knows` scrobble."
- "Offset `90` is a real `Don't Let Me Down` scrobble."

## Preview Before Deleting

Always preview the chosen range first:

```bash
nix develop -c cargo run -- -q delete --recent-offset 1-89
```

Confirm that:

- The first and last scrobbles match the intended block.
- Known-good anchors are outside the range.
- The count looks plausible.

## Apply The Deletion

Use `--apply` to execute deletion:

```bash
nix develop -c cargo run -- -q delete --recent-offset 1-89 --apply
```

The CLI will ask for confirmation. Stay attached to the process and report the final JSON summary.

Expected success signal:

```json
{"type":"Summary","total_found":89,"successful_deletions":89,"failed_deletions":0,"dry_run":false}
```

## Notes

- Prefer `--recent-offset` for accidental recent playback sessions.
- Use `--timestamp-range` only when the user knows the exact time window better than the offset window.
- Re-check offsets if new real scrobbles arrive while you are inspecting the recent history; boundaries can shift.
- If `cargo run` fails outside the dev shell, retry with `nix develop -c`.

# MusicBrainz identity

## Problem

We want to correct scrobbles against MusicBrainz. That means two things: stripping packaging junk from titles ("Wild Horses - 2009 Re-Mastered Digital Version") and moving scrobbles from compilations and edition titles to the canonical studio album. An LLM decides the cases the evidence leaves open. Four things shape the design.

1. **MusicBrainz is slow.** At about 1 request per second, a full pass over roughly 25k artist/title pairs takes many hours. The planner runs providers inline inside the sync/plan/execute loop, so MusicBrainz calls cannot happen there.
2. **Recording ids alone don't prove "same performance".** The "Cocaine" on *The Cream of Clapton* is recording `aca688be`, and Slowhand's is `40ae4c06`. They are unmerged duplicates that share the ISRC `NLF057790024`. A rule of "move only when the recording id matches" would almost never fire on compilations.
3. **Search snippets are partial.** Their release dates are unreliable, and a lookup with `inc=releases` caps at 25 releases. Picking the canonical album needs the paged `release?recording=` browse.
4. **Seams to keep.** Subjects, the one-open-intent-per-subject rule, the event-sourced queue, and the executor as the only last.fm writer all stay. The code has to build for Android, and anything unproven goes to human review.

## Usage

```rust
let mut identity = Identity::open(IdentityConfig {
    state: state.clone(),
    cache_dir: dirs::cache_dir().unwrap().join("scrobble-scrubber"),
    http: Arc::new(http_client()),
    user_agent: UserAgent::new("scrobble-scrubber/0.2", "you@example.com")?,
    llm: LlmConfig::from_env(),
}).await?;

let planner = Planner::new(store.clone(), state.clone())
    .with_provider(identity.provider())
    .with_provider(RewriteRulesScrubActionProvider::from_rules(rules).with_identity(identity.reader()));

let lane = identity.lane(store.clone()).await?.with_event_bus(bus.clone());
tokio::spawn(lane.run(generation.clone()));
```

```toml
# ~/.config/scrobble-scrubber/config.toml
[identity]
enabled = true                  # run the resolving lane inside `run`
contact = "you@example.com"     # MusicBrainz asks clients for contact info
llm = "claude-cli"              # adjudicate with `claude -p` on your subscription
llm_research = true             # let it research unresolved keys on the web (default with claude-cli)
llm_research_budget = 20        # research calls per day

[identity.moves]                # each "off", "review" or "auto"
compilations = "auto"          # greatest hits / compilations -> the original album
editions = "auto"              # "X (Deluxe Edition)" -> "X"
singles = "review"             # single / EP -> the album (never auto)
```

Changing `[identity.moves]` needs no MusicBrainz traffic. The provider reports the policy as its generation, so the next plan re-plans every resolved key under the new setting. "auto" only auto-applies proven moves; weaker evidence still goes to review.

The LLM defaults to `claude -p` on that CLI's own login, so a Claude subscription works and no API key is needed. Set `llm = "claude-cli"` under `[identity]` (and optionally `llm_model`, default `opus`). It runs with no tools, settings, MCP servers or session history, from the temp directory, and `--json-schema` holds the answer to the allowed option ids. The `SCRUBBER_LLM_*` environment variables override the config. `SCRUBBER_LLM_PROVIDER=command` with a JSON argv in `SCRUBBER_LLM_COMMAND` runs any other harness CLI (`{schema}`, `{system}` and `{model}` are substituted; the case arrives on stdin). `anthropic` and `openai` call those HTTP APIs with a key. Research needs a command backend: `claude-cli` runs `claude -p` with only WebSearch and WebFetch, and `command` takes its argv from `SCRUBBER_LLM_RESEARCH_COMMAND`.

```text
$ scrobble-scrubber identity resolve --budget 50
$ scrobble-scrubber identity status
$ scrobble-scrubber identity show "Eric Clapton" "Cocaine" --album "The Cream of Clapton"
```

## Shape

**Ownership.** Each piece of state has exactly one writer.

| State | Sole writer | Readers |
|---|---|---|
| `musicbrainz.sqlite`, a rebuildable cache of trimmed domain JSON keyed by request | `identity::mb`, inside the lane | the lane |
| `resolutions.jsonl` | `ResolutionWriter`, which is not `Clone` and is taken once by the lane | `ResolutionReader`: provider, rules, CLI, app |
| `queue.jsonl` | the planner and ops | the lane, which reads it for rejections and precedents |
| the change cursor and generation in `ProviderCoverage` | the planner | the planner |

The lane holds an exclusive lock on `<cache_dir>/lane.lock`, so only one process per machine writes the log and paces MusicBrainz. Once it holds the lock, it re-folds the log, so it never appends on top of a stale view. Change-feed positions are positions in the folding process's view of the log; a cursor past that view belongs to another process and is left alone.

**Data.**

- **`ScrobbleKey`** is `(artist, track, album)`. Album artist doesn't decide which recording a scrobble is.
- **`Outcome`** is self-contained: the matched master, the MB recording title, the scrobbled release group, the canonical release group, and a `Proof`. The proof is one of `SameRecording`, `SharedIsrc`, `SameWork`, `TitleOnly`, or `Researched`. Planning never reads the MB cache.
- **`ResolutionEvent`s** fold into one `KeyEntry` per key. Its status is `Settled`, `Open` (an ambiguous case plus an optional LLM verdict), `NotFound`, `Deferred`, or `Stale`. The entry also carries every target a human rejected for the key.

**Flow.**

1. **The lane resolves keys.** Keys with a trigger (multi-album titles or title junk) go first, then by play count.
2. **Resolution.**
   - Search with the parsed base title, anchored on the edition-stripped album. If the anchored search finds nothing, search without the album, then with the raw title. A search returns at most 100 hits. If MusicBrainz matched more than that, the outcome's proof drops to `TitleOnly`, because an unseen hit could be the better match.
   - Filter the hits by folded title, artist credit, and identity markers.
   - If the scrobbled release group is a plain Album, stop there. It never moves albums, and that is the common one-request path.
   - Otherwise, expand. Look up the recording, add its ISRC siblings (same base title, same version markers in title and disambiguation, overlapping artists, lengths within 5 seconds) to form the master, browse each member's official releases, and choose the canonical album.
   - A compilation-only recording with no ISRC link looks for a same-work studio recording instead. That produces `SameWork`, which always needs review.
   - Group the candidates by the edit they imply. One group means `Resolved`. Several mean `Ambiguous`.
3. **The LLM picks an option for ambiguous cases.** It chooses an option id, or keep, or unsure. Nothing it writes reaches last.fm; the strings come from MusicBrainz. Each answer is logged with the evidence hash, model, and prompt version, and the same evidence is never asked about twice. With no LLM configured, or after an unsure answer, the options are offered to the human one at a time through the normal approve/reject queue.
4. **Research is the last resort.** After resolution and the choice LLM, the lane researches the most-played keys still unsettled: not-found keys, cases the LLM was unsure about, and title-only matches of keys that have an album. See [Research](#research).
5. **Rejections feed back through the log.** The lane records each rejected identity intent as `HumanRejected`. That excludes the target from future proposals, and the new event puts the key back in the change feed, so the next option gets proposed.
6. **The planner stays pure.** `IdentityProvider` maps each subject's key to its plannable outcomes and derives at most one edit. The planner drains `changes_since(cursor)` after its time batches. A key resolved after its time range was planned is still planned exactly once, and `occupied` absorbs any overlap.

**Research.** An LLM with web search gets the scrobble, the user's other albums for the title, and what the resolver tried. It answers with a MusicBrainz recording id and release group id, not strings. Ids that aren't UUIDs count as no answer.

- **Verification.** `Resolver::verify_research` looks the recording up and requires a fold-equal base title, consistent identity markers, and an artist credit that matches leniently ("Bill Evans" for "Bill Evans Trio"). The release group only picks among the recording's official release groups named like the scrobbled album. The outcome is then built by the resolver's own expand path, so canonical and move policy are unchanged.
- **Approval.** The proof is `Researched`. It plans as adjudicated, never auto-applies, and never confirms a rewrite rule.
- **The log.** Each answer is a `Researched` event with the confirmed outcome, the ids, the confidence, and the reason. A confirmed outcome settles the key; an unconfirmed one leaves the status alone. An answer that MusicBrainz errored while verifying is recorded as unconfirmed. Either way the key isn't researched again for 90 days.
- **Budget.** Research has its own daily budget, 20 calls by default. A failed call stops research for the round, and ordinary resolution goes on regardless.

**Policy.** This lives in pure functions: `canonical.rs`, `outcome.rs`, and `title.rs`.

- **Canonical album.** The canonical album is chosen from the official release groups of the master whose credit matches the recording. Preference runs plain Album, then Album+Soundtrack, then plain EP, then a same-titled Single. Within a rung, the earliest first-release date wins.
- **When an album moves.** A scrobble moves only from a Compilation, from an edition title within the same release group, or (with review) from a Single or EP. It never moves from Live, Remix, DJ-mix, Demo, Soundtrack, or Spokenword release groups.
- **Album artist.** Moves off a compilation, single or EP set the album artist from the canonical credit. Otherwise the executor would keep "Various Artists" on a scrobble moved off a compilation. An empty credit never gets written; that move goes to review instead. Edition fixes leave the album artist alone.
- **Bonus tracks.** A release group only counts as a recording's album if the recording appears on one of its releases dated within a year of the group's first release. A B-side that only reached a later deluxe reissue doesn't belong to the original album. When dates are missing, the album ranks as `UndatedAlbum`, which never auto-applies.
- **Your spelling wins.** When MusicBrainz's text folds equal to yours ("Who’s Next" vs "Who's Next"), the edit keeps your characters and only removes the junk.
- **Edition suffixes.** Only suffixes made entirely of edition vocabulary (deluxe, remastered, anniversary, collector's, and so on) count as noise. "(Acoustic Edition)" and "(Live Edition)" are kept. A stripped " - Single" or " - EP" only matches a single or EP release group.
- **Titles.** A title only changes to the MB recording title. Deterministic resolutions only strip packaging junk. Identity markers like Live, Remix, Single Version, Clean (also Edited, Amended, Censored), and feat. are never stripped. "Explicit" is packaging.
- **Auto-apply.** A proposal applies without review only when all of these hold:
  - it was chosen deterministically;
  - the proof is `SameRecording`, or `SharedIsrc` with lengths within 5 seconds;
  - any album move is an edition fix or a move off a compilation, onto a plain Album that wasn't preceded by a single;
  - any title change is packaging only.

  Everything else goes to review. `Approval::Auto` can only be constructed by `approval()`. A rewrite rule that requires MusicBrainz confirmation is confirmed only by a settled outcome with one of those two proofs.

**What it deliberately does not do.**

- It doesn't edit artists.
- It doesn't let the LLM author strings or rules.
- It doesn't browse whole artist discographies.
- It doesn't store last.fm MBIDs yet.

## Synthesis decision

Three designs were sketched in parallel and judged against a six-criterion rubric. The base is the "identity lane" design. It scored highest on disambiguation correctness, because its ISRC-clustered masters handle the Cream of Clapton case. It also had the cleanest coverage answer (the change feed) and the clearest single-writer ownership.

**Grafts from the "evidence ledger" design.**
- A same-work sibling search when there's no ISRC link. This closes a silent miss in the base.
- `Approval::Auto` that only `approval()` can construct.
- A `UserAgent` that refuses to exist without contact info.
- Setting the album artist from the canonical credit on moves.

**Grafts from the "artist catalog" design.**
- Ordering work so keys with a trigger come first.
- Falling back to a raw-title query so titles MusicBrainz carries verbatim aren't mangled.

**Changed from the base.**
- Human feedback flows through `HumanRejected` events. In the base, it was derived in the provider on every batch, and a rejection in the human walk never re-triggered planning.
- LLM re-adjudication after a rejection is dropped in favor of the human walk.
- The MB cache is a single key-value table instead of five normalized tables.
- Identity always compiles, with no `musicbrainz` or `openai` feature, because its only network seam is `http_client::HttpClient`.

**Rejected.**
- The catalog design's whole-discography crawl. It's worse for the long tail, and its identity rule failed the Cocaine case.
- The evidence ledger's withheld coverage claims. They force a full-history replan every cycle during backfill.

## Tradeoffs accepted

- **Two edits for some scrobbles.** We accept that a key resolved after the rules provider already cleaned it may get a second edit, in exchange for keeping rules fast and independent.
- **Occasional ISRC errors.** ISRC equivalence can be wrong when MB data entry is wrong. We accept that, mitigated by the length tolerance and never moving off live or remix albums, in exchange for auto-fixing the most common compilation case.
- **Plain albums never move.** We accept missing the rare studio-album reissue under a new title, in exchange for one request per key on the majority path.
- **Slower human walk.** We accept walking ambiguous options one at a time through approve/reject, in exchange for needing no new review UI.

## Open questions

- Should high-confidence LLM picks ever apply automatically once their acceptance rate is proven?
- Are single-to-album moves wanted at all?
- Should the store keep the last.fm MBIDs (`src/api.rs` drops them) as a free first guess?

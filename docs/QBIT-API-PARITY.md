# qBittorrent WebUI API parity

rustTorrent ships a qBittorrent WebUI API v2 compatibility layer
(`crates/librtbit/src/http_api/handlers/qbit_compat.rs`, mounted at
`/api/v2`) so that *arr apps and other qBittorrent integrations can talk to
it. This document describes how parity with upstream qBittorrent is measured
and enforced, and where we currently stand.

## Current standing (upstream WebAPI 2.16.2, 2026-08-30)

**54 of 93 in-scope endpoints routed (~58%)** — 22 full, 32 partial — plus 2
legacy aliases (`torrents/pause`, `torrents/resume`) that upstream removed in
WebAPI 2.11. 37 of the 130 upstream endpoints were descoped on 2026-08-30
(marked `out_of_scope`, enforced as unrouted): `search/*` (Indexarr covers
discovery), `rss/*` (native `/rss` + Indexarr are the RSS story), `clientdata/*`,
SSL parameters, and qBittorrent-internal app misc (email, cookies, API keys,
processInfo, getDirectoryContent).

Decisions of record: advertised `webapiVersion` stays **2.11.3** and the
rename-family bug is now fixed *forward* (see below); the remaining heavy engine
items (queueing, per-torrent rate limits, move-storage, reannounce, rename)
**will be built**, not stubbed — they account for most of the 46 still-`missing`
in-scope endpoints.

| Controller | Implemented / in scope | Notes |
|---|---|---|
| `auth` | 2 / 2 | login/logout with SID cookies |
| `app` | 6 / 10 | version info, `defaultSavePath`, minimal preferences (7 descoped) |
| `torrents` | 35 / 60 | lifecycle, categories, tags, trackers (add/remove/edit), reannounce, pieces, file prio, per-torrent limits, export, webseeds |
| `transfer` | 10 / 13 | info (real session limits) + session rate limits + alt-speed mode + session pause/resume |
| `sync` | 1 / 2 | `maindata` (full-update snapshots; no per-client rid deltas) |
| `torrentcreator` | 0 / 4 | native `/torrents/create` exists, unbridged |
| `log` | 0 / 2 | no persistent log ring buffer |
| `rss`, `search`, `clientdata` | — | descoped entirely |

### Rename-family bug (fixed 2026-08-30)

We advertise `webapiVersion` **2.11.3**, so modern clients (qbittorrent-api,
newer *arr releases) use the post-2.11 vocabulary. This is now served:

- `torrents/stop` / `torrents/start` are routed (sharing the pause/resume
  handlers); the pre-2.11 `torrents/pause` / `torrents/resume` remain as aliases.
- `torrents/info` emits the 2.11 state strings `stoppedDL` / `stoppedUP`.
- `matches_filter` accepts `stopped` / `running` (and still the old
  `paused` / `resumed`), so `filter=stopped` no longer returns every torrent.
- `torrents/add` reads the 2.11 `stopped` form field as well as `paused`.

### Conflict audit (2026-08-30)

No hard route conflicts: the native API owns `/` and the compat layer owns
`/api/v2`; nothing native is mounted under `/api/*`, so all 111 missing
endpoints have free paths. Auth layering is also sound — the main
Bearer/Basic middleware is applied before the qbit router is nested, so
`/api/v2` correctly runs its own SID-cookie auth against the same credential
store. **Trap**: future qbit sub-routers (`sync`, `rss`, `log`, …) must be
nested inside `protected_router` in `make_qbit_router`, or they ship
unauthenticated.

The version-skew conflicts (items 1–5) were the 2.11 pause→stop rename family
and the rate-limit state disagreement; all are now resolved:

1. ~~**`torrents/stop`/`start` missing**~~ — routed (see the rename-family
   section above).
2. ~~**State strings** `pausedDL`/`pausedUP`~~ — `torrents/info` now emits
   `stoppedDL`/`stoppedUP`.
3. ~~**Filter values** `stopped`/`running`~~ — `matches_filter` now recognises
   both the 2.11 and pre-2.11 spellings, so `filter=stopped` no longer returns
   every torrent.
4. ~~**`torrents/add` `stopped` field**~~ — now read alongside `paused`.
5. ~~**Rate-limit state disagreement**~~ — `transfer/downloadLimit`/
   `uploadLimit`/`setDownloadLimit`/`setUploadLimit` now read and write the same
   session limiter as the native `/torrents/limits` API, and
   `transfer/speedLimitsMode`/`toggleSpeedLimitsMode`/`setSpeedLimitsMode`
   reflect the native `/speed/alt` toggle. `transfer/info` now also reports the
   real session `dl_rate_limit`/`up_rate_limit` (was hardcoded to 0).

Still open:

6. **`app/setPreferences` rejects instead of ignoring**: it uses
   `deny_unknown_fields`, returning 400 for any field other than
   `announce_port`. Real qBittorrent ignores unknown fields and applies the
   rest, so any client that round-trips preferences (get → modify → set)
   fails hard.
7. **RSS data-model gap** (affects future bridging): the native store is flat
   (feeds keyed by name, rules with one feed + one regex); qBittorrent has
   folder hierarchies (`Folder\Feed` paths) and rules keyed by name with
   `mustContain`/`mustNotContain`/`affectedFeeds[]`. A bridge needs flat-folder
   emulation and a rule-shape mapping; `rss/addFolder`/`rss/moveItem` have no
   clean mapping.

Hygiene note (adjacent): `QbitSessions` never purges expired SIDs except on
explicit logout, so the session map grows unboundedly with clients that
re-login frequently (Sonarr does).

### Suggested parity tiers

Tier 1 (high value) and the thin backend bridges from tier 2 are now largely
done: `torrents/stop`/`start`, `sync/maindata`, `app/defaultSavePath`,
`torrents/tags` + tag CRUD, `torrents/trackers`, `torrents/addTrackers`,
`torrents/addPeers`, `torrents/filePrio`, `torrents/pieceStates`/`pieceHashes`,
`torrents/export`, `torrents/webseeds`, `torrents/count`, the `transfer/*Limit*`
+ speed-limits-mode family, and `transfer/pauseSession`/`resumeSession`.

Remaining work, roughly by cost:

1. **Needs a new engine method** (the `will be built` items): `torrents/rename`
   / `renameFile` / `renameFolder`, `torrents/setLocation` / `setSavePath`,
   queueing (`topPrio` / `bottomPrio` / `increasePrio` / `decreasePrio`),
   `torrents/setShareLimits`, `setSuperSeeding`, `toggleSequentialDownload`,
   `setForceStart`, `setAutoManagement`. Rename and set-location need a new
   storage-trait rename primitive plus a mutable file-path mapping (the metadata
   is currently immutable) — a deliberate design, not a quick bridge.
   Done since: per-torrent rate limits (`ratelimit_override` on
   `ManagedTorrentShared`, enforced by the live limiter) and `reannounce`
   (signals the live re-discovery notify).
2. **Thin bridges still open**: `torrentcreator/*` (native `/torrents/create`,
   needs a task-lifecycle store), `torrents/setComment`, web-seed mutation
   (`addWebSeeds` / `editWebSeed` / `removeWebSeeds` — `web_seed_urls` is
   currently immutable).
3. **Probably out of scope**: `search/*` (plugin system),
   `app/sendTestEmail`, `app/processInfo`, `clientdata/*`,
   `torrents/SSLParameters`, `log/*`.

## The parity checker framework

The source of truth is
`crates/librtbit/src/http_api/handlers/qbit_parity_spec.json`: one entry per
upstream endpoint (`controller/action`), with its HTTP method and a parity
status:

- `full` — routed, semantics close enough for real clients
- `partial` — routed, but with stubbed fields or gaps (see `notes`)
- `missing` — not routed; candidate for future work
- `out_of_scope` — not routed by explicit decision

Entries with `"upstream": false` are legacy endpoints we serve that upstream
has removed.

Tests in `crates/librtbit/src/http_api/handlers/qbit_parity.rs` enforce the
spec against the real router (they run in the normal
`cargo test --workspace` CI job):

- `spec_matches_router` — probes every spec entry against
  `make_qbit_router()`; `full`/`partial` must be routed, `missing`/
  `out_of_scope` must 404. Implementing or removing an endpoint without
  flipping its spec status fails CI.
- `every_compat_route_is_tracked_in_spec` — scans the route literals in
  `qbit_compat.rs` so no compat route can land untracked.
- `parity_summary` — prints the scoreboard
  (`cargo test -p swarmforge qbit_parity -- --nocapture`).

### Testing scope (the parity checker is tier 0, not the whole story)

The tests above enforce *surface* parity only — that an endpoint is routed,
not that its response is correct. Full durable testing is scoped as a ladder
(tracked as WI-25/WI-26):

1. **Shape conformance** (WI-25, mandatory before the endpoint wave): each
   `full`/`partial` spec entry gains a response schema; the parity tests probe
   endpoints with a live in-process torrent and validate JSON field
   sets/types.
2. **Behavioral lifecycle** (WI-25): add→stop→start→recheck→delete suites
   through the compat router against a real session.
3. **Client-replay fixtures** (WI-26): recorded Sonarr / qbittorrent-api
   request sequences replayed against the router.
4. **Differential harness** (WI-26, nightly): identical requests against a
   real qBittorrent container and rustTorrent, responses diffed field-by-field
   with an allowlist for intentionally-stubbed fields.

Definition of done for every endpoint: route + spec status flip + schema +
lifecycle coverage.

### Workflow

- **Implementing an endpoint**: add the route, flip the spec entry to
  `full`/`partial`, note any semantic gaps in `notes`.
- **Declaring non-goals**: set status to `out_of_scope` (kept enforced as
  unrouted).
- **Tracking upstream**: re-clone qBittorrent and run
  `scripts/refresh-qbit-parity-spec.py <checkout>` — it re-extracts the
  endpoint inventory and POST allowlist from the sources, preserves our
  statuses/notes, adds new upstream endpoints as `missing`, and flags
  upstream-removed endpoints.

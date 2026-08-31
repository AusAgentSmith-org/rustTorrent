# qBittorrent WebUI API parity

rustTorrent ships a qBittorrent WebUI API v2 compatibility layer
(`crates/librtbit/src/http_api/handlers/qbit_compat.rs`, mounted at
`/api/v2`) so that *arr apps and other qBittorrent integrations can talk to
it. This document describes how parity with upstream qBittorrent is measured
and enforced, and where we currently stand.

## Current standing (upstream WebAPI 2.16.2, 2026-08-30)

**59 of 93 in-scope endpoints routed (~63%)** — 23 full, 36 partial — plus 2
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
| `torrents` | 40 / 60 | lifecycle, categories, tags, trackers (add/remove/edit), reannounce, rename (name/file/folder), setLocation/setSavePath, pieces, file prio, per-torrent limits, export, webseeds |
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

1. **Needs a new engine method** (the `will be built` items): queueing
   (`topPrio` / `bottomPrio` / `increasePrio` / `decreasePrio`),
   `torrents/setShareLimits`, `setSuperSeeding`, `toggleSequentialDownload`,
   `setForceStart`, `setAutoManagement`, `setDownloadPath` (incomplete-file
   path). Done since: per-torrent rate limits (`ratelimit_override` on
   `ManagedTorrentShared`, enforced by the live limiter), `reannounce` (signals
   the live re-discovery notify), **file/folder/display rename**, and
   **`setLocation`/`setSavePath`** (whole-torrent relocation — see below).

### Relocation (setLocation, v1, 2026-08-31)

`torrents/setLocation` and `torrents/setSavePath` move every file to a new root:

- New `TorrentStorage::move_root(new_root)` primitive: the filesystem backend
  moves each file to the same relative path under the new root (reusing the
  no-clobber rename core) and re-anchors its (now `RwLock`-wrapped) root; mmap
  forwards. All-or-nothing with rollback.
- The persistent anchor is a new `output_folder_override` on
  `ManagedTorrentShared` (seeded from `options.output_folder`, read by the
  storage factory's `create`), rather than unfreezing the immutable `options`.
- `ManagedTorrent::set_location()` is **stopped-only** (409 when live) and
  **same-filesystem only** — a cross-device `rename` (EXDEV) is refused and
  rolled back, not copied. `qbit_save_path` reports the new root after a move.
- **v2**: cross-filesystem relocation (async copy+delete, à la
  `move_completed_torrent`), live relocation, and persistence across restart.

> **⚠️ Restart caveat (rename and relocation both).** These overrides are
> in-memory only. Persistence still records the add-time `output_folder` and
> derives filenames from the immutable `.torrent` info, so after a
> rename/relocation **and a restart** the torrent re-adds at its original root
> with original names while the data sits at the new location — a recheck finds
> nothing and re-downloads, orphaning the moved copy. Until v2 persistence
> lands, treat rename/relocation as effective only within the running session.

### File rename (v1, 2026-08-31)

`torrents/renameFile`, `renameFolder`, and `rename` (display name) are
implemented:

- New `TorrentStorage::rename_file(file_id, new_relative)` primitive: the
  filesystem backend moves the file on disk and re-points its cached open
  handle at the new path (mmap forwards; other backends default to an error).
- `ManagedTorrent::rename_files()` is **stopped-only** (returns 409 when live,
  which sidesteps live-handle / Windows-open-file / mmap-remap hazards): it
  validates the batch (relative paths, no `.`/`..`/root, no collisions),
  moves each file, then swaps in rebuilt `TorrentMetadata` with updated
  `file_infos` and prunes emptied source dirs. All-or-nothing with rollback.
- `torrents/rename` sets a `name_override` on `ManagedTorrentShared`, surfaced
  in `torrents/info` / `sync/maindata` via `ManagedTorrent::name()`.
- **v1 limitations**: renames are not persisted across restarts (re-derived
  from the immutable `.torrent` info on load), and require the torrent stopped.
  Live rename is the documented v2, gated behind the differential-test harness.
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

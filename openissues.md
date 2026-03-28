## Issue Review: ikatson/rtbit (62 open)

Reviewed: 2026-03-20 (verified against codebase and git history, updated after commit `abb0af59`)

### Fixed in This Fork

| # | Title | Fix | Commit |
|---|-------|-----|--------|
| **#525** | Memory leak / FD exhaustion | Dead peer pruning (60s GC, 5min TTL), accept loop cap (64 concurrent handshakes), bounded peer queue (10k), default peer limit (200/torrent) | `d0389801`, `e7a0fe82` |
| **#311** | Connection exhaustion (20k WAIT-CLOSE) | Same resource management overhaul as #525 | `d0389801`, `e7a0fe82` |
| **#561** | Checking pauses after disconnect, doesn't resume | Per-torrent cancellation token, pause-during-init support | `ae855766` |
| **#347** | Adding torrent hangs at 0% "checking files" | Magnet resolution timeout (120s, configurable), init queue feedback (`queued_for_init` flag), per-torrent cancellation | `85bf17b7`, `ae855766` |
| **#348** | Deleting torrent doesn't cancel "add torrent" API call | Per-torrent cancellation token cancelled before pause on delete; file verification checks cancellation per-piece | `ae855766` |
| **#363** | Download fully stops on temporary internet disruption | Peer health monitor (30s interval), dead peer re-queueing, `rediscovery_notify` triggers fresh DHT/tracker, frozen peer retry (600s) | `fbea4aa6` |
| **#477** | Out of range integral type conversion | Safe `TryFrom`/`TryInto` conversions throughout (file offsets, ETA calcs, timestamps, piece indices) | `02a00598` |
| **#267** | Support tags / *arr app compatibility | Full qBittorrent WebUI API v2 at `/api/v2` (auth, torrents, categories, transfer), WebUI category support | `166b8a16`, `123b65a7`, `baa0df7c` |
| **#482** | HTTP API authentication | Basic auth with constant-time comparison + Bearer token auth (access/refresh tokens, 15min/30day TTL). Persistent credential store (JSON file, 0o600 perms), first-boot setup flow, login/logout UI, password change in settings, token auto-refresh on 401 | `e7a0fe82`, `fa67ed99`, `abb0af59` |
| **#439** | Disable HTTP API by default (security) | `--disable-http-api` CLI flag and `http_api.disable` desktop config option | existing |
| **#484** | Don't create files for unselected torrent files | `update_only_files` API for dynamic file selection/deselection | existing |
| **#160 / #539** | Missing Stopped/Completed announce events | HTTP tracker now sends correct Started/Completed/Stopped events, mirroring UDP logic. Full event lifecycle in `tracker_comms.rs:245-284` | `fa67ed99` |
| **#493** | SOCKS proxy leaks (UDP trackers, DHT, etc.) | DHT, UDP trackers, and LSD auto-disabled when SOCKS proxy configured. Warnings logged for each disabled subsystem (`session/mod.rs:185-360`) | `fa67ed99` |
| **#349** | Fast resume not used when adding torrents | `previously_errored` logic fixed: `false` on normal restart (uses fastresume with probabilistic validation), `true` on error recovery (forces full recheck). Configurable validation denominator (default 50), at least 1 piece per file always validated | `e88dedaa`, `fa67ed99` |
| **#346** | Set RUST_BACKTRACE=1 by default | Set at program startup in `main.rs:484-486` if not already set | `fa67ed99` |
| **#352** | Performance issues with blur CSS effect | All `backdrop-blur` instances removed from WebUI | `fa67ed99` |

### Still Outstanding — High Priority

| # | Title | Current State |
|---|-------|---------------|
| **#70 / #546** | BEP 52 (BitTorrent v2) | **Phase 1 done**: Hybrid v1+v2 torrents work (v1 download path). `pieces` now optional, `meta_version` field parsed, v2-only torrents get clear error. Remaining phases: v2 info hash (SHA-256), file tree parsing, merkle tree verification, v2-only download. |
| **#369** | File locked by process (Windows .exe files) | Not addressed. Generic `lock_write`/`lock_read` used in `storage/filesystem/mmap.rs:112`, no Windows-specific handling. |

### Still Outstanding — Medium Priority

| # | Title | Current State |
|---|-------|---------------|
| **#575** | Upgrade reqwest to 0.13 | Still on reqwest 0.12 (`Cargo.toml:129`). |
| **#514** | Release new librtbit version | Not addressed. |
| **#350** | Limit memory usage | **Improved** — peer_limit and concurrent_init_limit now runtime-configurable via `POST /torrents/limits` API and GUI Settings > Speed tab. Env vars: `RTBIT_PEER_LIMIT`, `RTBIT_CONCURRENT_INIT_LIMIT`. Stored as AtomicUsize on Session for lock-free reads. No explicit process-level memory ceiling. |
| **#310** | Excessive DHT cache disk I/O | **Improved** — new `--dht-dump-interval` / `RTBIT_DHT_DUMP_INTERVAL` env var to control write frequency (default 60s, set higher e.g. `5m` to reduce I/O). |
| **#553** | Settings change triggers full restart + recheck | **Partially improved** — rate limits, peer_limit, and concurrent_init_limit are now runtime-configurable without restart (via API + GUI). Desktop app `configure()` still stops entire session for other settings. Full fix requires Session architecture refactor (interior mutability for DHT, listener, tracker options). |
| **#412** | Preallocation support | Not addressed. No `fallocate` or preallocation calls in storage code. |
| **#308** | Move files to different directory | Not addressed. |
| **#136** | Support read-only downloaded files | Not addressed. |

### Lower Priority — Nice to Have / Large Features

| # | Title | Notes |
|---|-------|-------|
| **#463** | BEP-55 Holepunch extension | **Implemented**: `ut_holepunch` extension (protocol layer + relay/connect handling). Rendezvous relay logic, Connect peer-add, Error responses. Private torrent guard. Requires uTP enabled to function. |
| **#385** | BEP-46 Updating torrents via DHT | **Foundation done**: BEP 44 DHT get/put mutable item messages (serialize/deserialize), Ed25519 crypto (sign/verify via aws-lc-rs), MutableItemStore (thread-safe, seq-ordered, capacity-limited). Magnet link `xs`/`s` parsing. 88 DHT tests pass (32 new). Remaining: iterative get_mutable/put_mutable lookup, session subscription/polling, API endpoints. |
| **#500** | Webseed support | **Phase 1 done**: `url-list` field parsed from torrent metainfo (handles single string and list of strings). URLs threaded to ManagedTorrentShared and logged on torrent start. Phase 2 (HTTP range download task) not yet implemented. |
| **#71 / #12** | WebRTC / WebTorrent | Not implemented. No WebRTC/WASM references in codebase. |
| **#457 / #361** | Tor / I2P support | SOCKS5 proxy implemented (`stream_connect.rs`), but no native I2P/Tor client. |
| **#491** | Native UI alternative | 10 comments. Users want GTK/Qt instead of web. |
| **#313** | Localization | Not implemented. No i18n framework or language files. |
| **#440 / #318** | System tray minimize (Windows/Linux) | Not implemented. Tauri provides tray API but not integrated. |
| **#550** | QoS / niceness / priorities | Not implemented. No file priority support (only `only_files` selective download). |
| **#551** | Import from other clients | Partial — magnet links, .torrent files, and HTTP URLs supported. No migration from qBittorrent/Transmission configs, but qBit API v2 compat allows *arr apps to use rtbit as drop-in replacement. |
| **#425** | UPnP/DLNA controller | Implemented — full UPnP server in `crates/upnp*/`, SSDP discovery, Content Directory, Connection Manager. Desktop config: `upnp.enable_server`. |

### Can Probably Close

| # | Title | Notes |
|---|-------|-------|
| **#276** | How to set DHT listen_addr in Docker | Question, not bug. Likely answered. |
| **#306** | Will timeout with add_torrent cause issues? | Question. |
| **#473** | How to set custom User-Agent? | Question, 5 comments — likely resolved. |
| **#509** | How to seed pre-downloaded torrent? | Question, 4 comments. |
| **#327** | Example with resumable torrent | Question/docs, 8 comments. |
| **#354** | Store output_folder relative to home | Very niche. |
| **#343** | Unable to download specific torrent | Single torrent issue, may be tracker-side. |
| **#487** | 8.x releases larger than 9.x | Likely expected (different build config). |
| **#471** | Dependency issue with crates.io | Related to #514, may be resolved. |

### Recommendations — What to Fix Next

**Quick wins** (high impact, low effort):
1. ~~**#539/#160** — HTTP tracker announce events.~~ Done.
2. ~~**#346** — RUST_BACKTRACE=1 default.~~ Done.
3. ~~**#352** — CSS blur perf.~~ Done.
4. ~~**#493** — Proxy leaks.~~ Done.
5. ~~**#349** — Fast resume skip on fresh adds.~~ Done.

**Remaining priorities:**
1. **#70/#546** — BEP 52 Phase 2-5: v2 info hash (SHA-256), file tree parsing, merkle verification, v2-only download.
2. **#500** — Webseed Phase 2: HTTP range download task, piece verification, integration with PieceTracker. url-list parsing already done.
3. **#385** — BEP-46 Phase 2: iterative get_mutable/put_mutable DHT lookup, session subscription/polling, API endpoints. Foundation (BEP 44 protocol + crypto + store) already done.
4. **#553** — Settings hot-reload for remaining options (DHT, listener, trackers). Architectural change needed.
5. **#369** — Windows file locking. Platform-specific investigation needed.
6. **#575** — reqwest 0.12 -> 0.13 upgrade.
7. **#412** — File preallocation. Would improve performance on HDDs.
8. **#350** — Process-level memory ceiling. Runtime peer/init limits now work but no hard memory cap.

**Fork-specific enhancements not in upstream issues:**
- Benchmark suite (benchv2) for performance regression testing
- Full qBittorrent WebUI API v2 compatibility
- Bearer token authentication with persistent credential store
- First-boot setup flow (username/password creation required)
- Login/logout UI with token auto-refresh on 401
- Password change in settings dialog (Security tab)
- Runtime-configurable peer_limit and concurrent_init_limit (API + GUI)
- WebUI: drag-and-drop, multi-torrent upload, resizable columns, context menus, categories, compact view
- BEP 55 holepunch extension (ut_holepunch relay/connect)
- BEP 44 DHT mutable items foundation (Ed25519, MutableItemStore)
- BEP 19 webseed url-list parsing
- BEP 52 v2 hybrid torrent support (Phase 1)
- UPnP/DLNA server
- Prometheus metrics
- Configurable DHT dump interval (`RTBIT_DHT_DUMP_INTERVAL`)

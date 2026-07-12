# rustTorrent — Architecture & BEP Review (upliftv2)

**Date:** 2026-07-13
**Scope reviewed:** `crates/rtbit` (CLI/server binary) plus the engine and supporting
crates it consumes from the Forgejo registry — `librtbit`, `librtbit-peer-protocol`,
`librtbit-dht`, `librtbit-tracker-comms`, `librtbit-lsd`, `librtbit-bencode`
(cloned from `repo.indexarr.net/indexarr/*` at their current tips).
**Method:** static source review of the binary + libraries. No live swarm test was run,
so peer-wire and tracker findings are read from code, not captured on the wire.

---

## 0. TL;DR

- **Architecture: fit for purpose.** The core engine (`librtbit`) is a mature,
  well-structured async BitTorrent stack (it is a rebrand/fork of Igor Katson's
  `rqbit` — the crate authorship and module layout confirm this). Tokio-based,
  cleanly separated into session / torrent-state / peer / storage layers, with
  pluggable storage, persistence backends (JSON + Postgres), and a broadcast-based
  stats pipeline. This is a solid foundation.
- **The risk is not the core — it's the bespoke layers bolted on top.** The
  qBittorrent-compatibility shim, the auth/credential system, RSS, and Indexarr
  integration are locally written and are where the real correctness and security
  issues live.
- **BEP coverage is good for a modern DHT/magnet client but has real gaps:**
  no Fast Extension (BEP 6), no Message Stream Encryption (MSE/PE), no DHT `PORT`
  peer message (BEP 5's in-band port exchange), no tracker scrape (BEP 48), and
  the handshake reserved bits advertise only extended messaging.
- **Highest-priority defects:** a qBittorrent-API auth bypass, an unconditional
  "always unchoke, never choke" upload model, and several *arr-integration
  correctness bugs (category filtering ignored, seed/peer counts hardcoded).

---

## a) Is the design architecturally fit for purpose?

**Yes, with caveats.** Verdict by layer:

### Core engine — strong
- **Clean layering.** `session` → `torrent_state` (initializing / live / paused) →
  `live/peer*` → `peer_connection` → `peer-protocol`. Storage is abstracted behind
  a `StorageFactory` trait with filesystem and mmap implementations plus middleware
  hooks (timing/slow-disk) for debugging. Persistence is abstracted (`Json` /
  `Postgres`) behind `SessionPersistenceConfig`.
- **Async model is sound.** Tokio multi-thread runtime, `CancellationToken`-based
  graceful shutdown, a dedicated blocking spawner for disk I/O, dual-stack
  (IPv4/IPv6) sockets via `librqbit-dualstack-sockets`, SOCKS5 proxy support, and
  optional uTP.
- **Operational maturity.** UPnP port-forwarding + optional UPnP MediaServer,
  systemd socket activation, Prometheus metrics, blocklist/allowlist support,
  rate limiting, file-watch folder, completed-folder move, `nofile` limit bump.
  These are the marks of a client that has actually been run in production.

### Bolt-on layers — mixed
- **qBittorrent compat (`qbit_compat.rs`, ~1170 lines):** functional but shallow.
  Many fields are hardcoded/stubbed (peer counts, ratios, seeding time, categories),
  and it re-implements its own session/cookie auth separately from the main auth
  stack — which is the source of the security gap in section (c).
- **Auth (`http_api/auth.rs`):** two parallel systems — a token store (bearer +
  refresh, in-memory) and a credential store (file-backed). The main API is guarded
  by one middleware; the qBit API is guarded by a *different*, weaker one. Two
  overlapping auth models on one server is an architectural smell and already leaks
  (see c-1).
- **RSS + Indexarr:** self-contained (SQLite-ish `rss.db`, a polling monitor task).
  Reasonable, but the monitor is spawned unconditionally at startup and its failure
  modes are only logged.

### Architectural concerns worth addressing
1. **Hard fork of rqbit — declared, not tracked.** The whole stack is a fork of
   `rqbit` (github.com/ikatson/rqbit) republished under `librtbit-*` on a private
   registry. **Decision (2026-07-13): rustTorrent will NOT pull any further changes
   from upstream — this is a permanent hard fork.** That is a valid choice and
   simplifies maintenance (no rebase machinery, free to refactor), but it carries
   consequences that must be owned explicitly:
   - **You now own the entire security surface.** Any latent bug already present in
     rqbit's code — especially the untrusted-input parsers (`librtbit-bencode`
     decode, `librtbit-peer-protocol` message deserialize, `librtbit-dht`
     `bprotocol`) — is yours to find and fix. A panic on malformed input from a
     remote peer is a remote DoS.
     **Action:** stand up `cargo fuzz` targets on those three parsers; this is the
     highest-value hardening step post-fork.
   - **Awareness, not merging.** Monitor upstream advisories/changelog read-only so
     that if ikatson patches a class of bug (DHT amplification, bencode DoS), you can
     hand-port the *fix idea* into your own code. No dependency, just situational
     awareness.
   - **License attribution is still mandatory.** rqbit is Apache-2.0/MIT; keep the
     original copyright and license headers even in a hard fork. Today the crate
     `authors` still reads "Igor Katson" and `repository` points at
     `github.com/ikatson/rtbit` — update the fork's own metadata while preserving
     the upstream copyright notices.
   - **Record the fork point once** (a one-line note per crate README with the
     upstream commit hash) for provenance, then move on.
2. **Two auth subsystems on one listener** (main API vs qBit API) — should be unified
   behind a single middleware.
3. **In-memory-only token/session stores.** All bearer tokens and qBit `SID`
   cookies are lost on restart (`HashMap` in `RwLock`, no persistence). Acceptable
   for tokens; mildly annoying for long-lived *arr sessions, which must re-login.
4. **README performance claims ("15x faster than qBittorrent", "16+ Gbps").** These
   are throughput/disk numbers, not swarm-behaviour numbers. Given the "always
   unchoke, no choking algorithm" model (section b/c), real-world swarm performance
   and fairness will not track those benchmark figures. Make sure marketing claims
   are scoped to the LAN/seedbox scenarios they were measured in.

---

## b) Are BitTorrent BEP standards honored?

Mixed — good magnet/DHT/extension coverage, but several notable omissions.

### Implemented / honored
| BEP | Feature | Status | Evidence |
|-----|---------|--------|----------|
| 3 | Core peer wire protocol | ✅ | `Message` enum: choke/unchoke/interested/have/bitfield/request/piece/cancel |
| 5 | DHT (Kademlia) | ✅ (partial) | `librtbit-dht`: `get_peers`/`announce_peer`/`find_node`, token handling, rate limiting |
| 9 | Metadata exchange (`ut_metadata`) | ✅ | `extended/ut_metadata.rs` |
| 10 | Extended messaging | ✅ | Handshake sets reserved bit `1<<20`; `Extended` message + extended handshake |
| 11 | Peer exchange (`ut_pex`) | ✅ | `extended/ut_pex.rs` |
| 12 | Multi-tracker | ✅ | Tracker tiers / multiple announce URLs |
| 15 | UDP tracker (announce) | ✅ (announce only) | `tracker_comms_udp.rs`, connect+announce |
| 19 | WebSeed (HTTP/URL-list) | ⚠️ referenced | `url-list`/`WebSeed` mentioned; verify end-to-end |
| 23 | Compact peer lists | ✅ | `compact=1`, `CompactListInBuffer` |
| 29 | uTP | ✅ (experimental) | `--experimental-enable-utp-listen` |
| 32 | DHT IPv6 (`n6`/`nodes6`) | ✅ | `bprotocol.rs` `Want::V6/Both`, `nodes6` |
| 44 | DHT store (mutable/immutable) | ✅ | `bprotocol.rs` Bep44 handling |
| 55 | Holepunch (`ut_holepunch`) | ✅ | `extended/ut_holepunch.rs` |

### Not honored / missing (findings)
| BEP | Feature | Impact |
|-----|---------|--------|
| **6** | **Fast Extension** (HaveAll/HaveNone/Suggest/RejectRequest/AllowedFast) | **Missing entirely.** Wire enum has no fast messages and the handshake never sets the fast bit (`0x04`). Costs a fast-start optimisation and interop nicety; some peers prefer fast-capable clients. |
| **PE/MSE** | **Message Stream Encryption / protocol obfuscation** | **Missing entirely.** No RC4/DH negotiation anywhere. Many **private trackers require or prefer encryption**, and some ISPs throttle plaintext BT. This is the single biggest interop gap for real-world/private-tracker use. |
| **5** | DHT **`PORT` peer message** (in-band DHT port exchange) | Missing (`MSGID_PORT` = 9 absent; handshake never sets DHT reserved bit `0x01`). DHT still bootstraps and learns nodes via PEX, so it works, but you don't advertise/learn DHT nodes over existing peer connections and don't signal DHT support in the handshake. README claims "Full DHT support (BEP-5)" — the in-band port message part of BEP 5 is not implemented, so soften that claim. |
| **48** | Tracker **scrape** (and UDP scrape) | `ACTION_SCRAPE` is commented out in `tracker_comms_udp.rs`; no HTTP scrape. Seeders/leechers counts from trackers are therefore unavailable — this feeds directly into the hardcoded-zero peer counts in the qBit layer (c-4). |
| **42** | DHT security extension (node-ID derived from IP) | Not implemented. Lower priority; improves DHT resistance to poisoning. |
| **7 / 24** | IPv6 tracker / peer address extensions | IPv6 works via dual-stack sockets and DHT `n6`, but verify tracker `&ipv6=`/compact6 handling. |

### Choking algorithm (BEP 3 §"choking and optimistic unchoking")
**Not honored.** In `peer_connection.rs` the client sends `Message::Unchoke` to every
peer **unconditionally, immediately after the handshake/bitfield, and never chokes
again.** There is:
- no tit-for-tat reciprocation,
- no optimistic-unchoke rotation,
- no fixed number of upload slots,
- no snubbing.

BEP 3 explicitly specifies a choking algorithm (typically 4 reciprocating slots + 1
rotating optimistic unchoke) as the swarm's fairness/incentive mechanism. rtbit's
"unchoke everyone forever" model is simpler and can look faster in a cooperative LAN
benchmark, but in a real swarm it (a) surrenders the upload-reciprocation lever that
gets you unchoked faster by peers, and (b) makes rtbit a poor swarm citizen under
upload contention. This is both a BEP-fidelity gap and an architecture issue. See
c-2.

---

## c) Bugs

Ordered by severity. File references are within the `librtbit` engine crate unless noted.

### c-1 (High, security) — qBittorrent API auth bypass when creds come from the credential store
`http_api/mod.rs` applies the main auth middleware via `route_layer` **before** the
qBit router is nested (`main_router.nest("/api/v2", qbit_router)` happens after the
`route_layer` call), so `/api/v2/*` is **not** covered by the main auth middleware.
The qBit router instead guards itself, but only when `basic_auth.is_some()`:

```rust
// qbit_compat.rs make_qbit_router
let has_auth = api_state.opts.basic_auth.is_some();
if has_auth { /* attach SID-cookie middleware */ }
```

The normal WebUI setup flow stores credentials in the **credential store**
(`credentials.json`), which leaves `basic_auth = None`. Result: **after a user sets
up a username/password through the web UI, the entire qBittorrent API
(`/api/v2/torrents/{add,delete,pause,…}`) is unauthenticated** and will happily
add/delete torrents for anyone who can reach the port. Also, `h_auth_login` returns
`auth_ok = true` whenever `basic_auth` is `None`, so even the qBit "login" rubber-stamps.
Fix: gate qBit auth on *any* configured credential source (env basic-auth **or**
credential store), and ideally route the qBit layer through the same unified
middleware.

### c-2 (High, behaviour) — no choking algorithm; permanent unchoke
As described in section (b). `peer_connection.rs:333` sends `Unchoke` once and there
is no code path that ever sends `Message::Choke` to a remote peer. Implement a
standard choker (reciprocation slots + optimistic unchoke on a ~10s/30s timer, upload
slot cap) — or at minimum a fixed upload-slot limit — to restore swarm fairness and
protect upload bandwidth.

### c-3 (Medium, *arr integration) — `category` query filter ignored in `/torrents/info`
`h_torrents_info` reads `query.filter`, `query.hashes`, `query.sort` but **never uses
`query.category`** (the field is even under `#[allow(dead_code)]`). When Sonarr/Radarr
poll `?category=tv-sonarr`, rtbit returns **all** torrents regardless of category.
Combined with the fact that `setCategory` is a silent no-op and `torrents/info` always
reports `category: ""`, any multi-*arr / multi-category setup will see each app
managing torrents that belong to another app. This is a real data-integrity risk for
Sonarr/Radarr users (wrong imports, wrong deletions).

### c-4 (Medium, *arr integration) — hardcoded peer/seed statistics
In `h_torrents_info`/`h_torrents_properties`:
- `num_complete`, `num_incomplete`, `num_leechs` are hardcoded `0`.
- `num_seeds` is set to `peer_stats.live` (total connected peers, **not** actual seeds).
- `peers`, `peers_total`, `seeds`, `seeds_total`, `pieces_have` in properties are `0`.
- `ratio`, `uploaded_session`, `seeding_time`, `time_active` are `0`/stubbed.

*arr apps use seed/peer counts and ratio for seeding-goal and health decisions, so
these zeros can cause premature removal or "no peers" health warnings. Root cause is
partly the missing tracker scrape (BEP 48). At minimum, populate `num_seeds`/leechers
from real connected-peer seed/leech classification rather than total live peers.

### c-5 (Medium, correctness) — synthetic timestamps break *arr history/seeding logic
`added_on`, `completion_on`, `last_activity`, `seen_complete`, `addition_date`, etc.
are all set to `now_unix()` on every poll rather than the torrent's real add/completion
time (these aren't persisted). Sonarr/Radarr compute seeding time and age from these;
returning "now" every request means seeding-time goals may never appear satisfied (or
appear satisfied instantly, depending on field). Persist real add/completion times in
session state and surface them here.

### c-6 (Low) — `/app/preferences` reports misleading globals
`h_app_preferences` returns `save_path` = the **first torrent's** output folder
(empty string if there are no torrents) and a hardcoded `web_ui_port: 3030` even when
the API is bound elsewhere. *arr uses `save_path` to validate its download path; an
empty or wrong value can trip "download path" checks. Return the session's configured
default output folder and the actual bound port.

### c-7 (Low) — non-constant-time comparison in qBit login
`qbit_compat.rs h_auth_login` compares credentials with `==`
(`form.username == *expected_user && form.password == *expected_pass`), unlike the
main auth path which uses `constant_time_eq`. Minor timing side-channel; use the
constant-time helper for consistency.

### c-8 (Low) — `matches_filter` "resumed"/"stalled" heuristics are approximate
"resumed" is defined as "not paused", and "stalled" keys off a `< 0.001 Mbps`
threshold with `unwrap_or(true)` when live stats are absent — so a torrent with no
live snapshot is always reported "stalled". Edge-casey but can mislead UI filters.

### c-9 (Low) — CLI `--worker-threads` naming vs blocking pool
Not a bug per se, but note: the session's `runtime_worker_threads` is wired from the
CLI's `--max-blocking-threads` (default 8) and actually controls the **disk blocking
spawner** (`session/mod.rs:306`), while Tokio's real worker-thread count comes from
`-t/--worker-threads`. The naming invites confusion; consider renaming
`runtime_worker_threads` → `blocking_pool_size`.

---

## d) Areas for improvement

### Protocol / BEP (highest interop value)
1. **Add MSE/PE encryption.** Biggest real-world gap. Required/preferred by many
   private trackers and helps against ISP throttling. Without it rtbit is a
   non-starter on a large class of trackers.
2. **Implement the choking algorithm** (reciprocation + optimistic unchoke + upload
   slots). Restores swarm fairness and protects upload bandwidth; also makes the
   "faster than qBittorrent" story defensible beyond LAN benchmarks.
3. **Add tracker scrape (BEP 48, HTTP + UDP).** Unblocks real seeder/leecher counts,
   which cascades into fixing the hardcoded qBit stats (c-4) and *arr health.
4. **Add Fast Extension (BEP 6)** and set the corresponding handshake reserved bit.
5. **Implement the DHT `PORT` message (BEP 5)** and set the DHT handshake bit, or
   correct the README's "Full DHT support (BEP-5)" claim.
6. Consider **BEP 42** (secure DHT node IDs) for DHT robustness.

### Security
7. **Unify auth** into a single middleware covering `/`, `/api/v2`, and UPnP, keyed
   off any configured credential source; fix c-1 first.
8. **Persist/rotate secrets safely** and consider hashing stored passwords
   (`credentials.json` currently stores the password in plaintext with `0600` perms;
   at least document this, ideally store a salted hash and compare against it).
9. **Rate-limit auth endpoints** (`/auth/login`, qBit `/auth/login`) to blunt
   brute-force, and add a lockout/backoff.
10. Review **CORS** (`AllowHeaders::any()` + env-driven `CORS_ALLOW_REGEXP`) — a
    misconfigured regex is an easy foot-gun; validate/anchor it.

### *arr / qBittorrent-compat correctness
11. Fix category handling end-to-end: honor `?category=` filter, persist per-torrent
    category, and echo it back in `/torrents/info` (c-3). This is the difference
    between "works with one *arr" and "works with a real *arr stack".
12. Populate real timestamps and real seed/leech/ratio numbers (c-4, c-5).
13. Return a correct `save_path` and bound port in `/app/preferences` (c-6).

### Architecture / maintainability
14. **Own the hard fork deliberately** (decided 2026-07-13: no further upstream
    pulls). Concretely: (a) add `cargo fuzz` targets for the bencode decoder, the
    peer-message deserializer, and the DHT `bprotocol` parser — since you now own
    every remote-input path; (b) monitor rqbit advisories read-only for fix ideas to
    hand-port; (c) preserve rqbit's Apache-2.0/MIT attribution and update the fork's
    own `authors`/`repository` metadata (currently still points at ikatson); (d)
    record the fork commit per crate README for provenance.
15. **Consolidate the two auth token stores**; make session/token lifetime and
    persistence a deliberate choice rather than "in-memory, lost on restart".
16. **Tests:** the compat layer has a couple of unit tests (ETA/timestamp overflow)
    but no integration coverage of the qBit API against a real *arr client or a
    golden qBittorrent response. Add contract tests that assert the JSON shape *arr
    expects, and a swarm smoke test (seed↔leech between two rtbit instances) to catch
    choke/interest regressions.
17. **Marketing vs reality:** scope the throughput claims in the README to the tested
    scenarios and stop implying full BEP-5/swarm-optimality until the choker and
    encryption land.

### Nice-to-haves
18. WebSeed (BEP 19) — verify it actually downloads end-to-end (only string
    references were found).
19. Super-seeding (BEP 16) for initial-seed scenarios.
20. Per-torrent and per-category ratio/seed-time limits (there is already an open
    issue `0007-per-category-share-limits.md` — aligns with fixing category support).

---

## Appendix — what was inspected

- Binary: `crates/rtbit/src/main.rs` (CLI, runtime, session wiring, auth bootstrap).
- Engine `librtbit`: `http_api/{mod,auth}.rs`, `http_api/handlers/qbit_compat.rs`,
  `session/{mod,types}.rs`, `torrent_state/live/{peer_handler,tasks}.rs`,
  `peer_connection.rs`, `rss/*`.
- `librtbit-peer-protocol`: `lib.rs` (Message enum, handshake, reserved bits),
  `extended/{handshake,ut_metadata,ut_pex,ut_holepunch}.rs`.
- `librtbit-dht`: `dht.rs`, `bprotocol.rs`, `peer_store.rs`.
- `librtbit-tracker-comms`: `tracker_comms{,_http,_udp}.rs`.

Findings are from source reading; the peer-wire and *arr-integration items (b, c-1..c-6)
should be confirmed with a live swarm test and a real Sonarr/Radarr connection before
being treated as closed.

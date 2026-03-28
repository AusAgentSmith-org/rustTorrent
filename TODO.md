# rustTorrent TODO — Feature Gap Analysis

Feature gap analysis based on qBittorrent, Deluge, and Transmission.
Items marked with the client initial(s) indicate which competitor(s) have the feature: **Q**=qBittorrent, **D**=Deluge, **T**=Transmission.

---

## Protocol & Encryption

- [ ] **#1 — BEP 52 Phase 2–5: Full BitTorrent v2 support** — v2 info hash (SHA-256), file tree parsing, merkle tree piece verification, v2-only torrent download. Hybrid v1+v2 works today; v2-only is rejected. [Q, D, T]
- [ ] **#2 — MSE/PE Stream Encryption (BEP 50/68)** — Message Stream Encryption / Protocol Encryption with configurable modes: allowed, preferred, required. [Q, D, T]
- [ ] **#3 — BEP 19 Phase 2: WebSeed HTTP downloads** — URL-list parsing is done; actual HTTP range-based piece fetching from web seeds is not. [Q, D, T]
- [ ] **#4 — BEP 46 Phase 2: DHT mutable item lookup** — Ed25519 foundation exists; actual mutable item resolution for updateable magnet links is not wired up.
- [ ] **#5 — Embedded tracker** — Run a lightweight tracker inside rtbit for private swarms. [Q]

## Transfer Management

- [ ] **#6 — Per-torrent rate limiting** — Currently only global rate limits exist. Allow setting upload/download speed caps per torrent. [Q, D, T]
- [ ] **#7 — Bandwidth scheduling / alternative speed limits** — Time-of-day speed profiles (e.g., "turtle mode" during work hours). [Q, D, T]
- [ ] **#8 — Super-seeding (BEP 16)** — Initial-seeder mode that serves each piece only once to maximize swarm distribution. [Q, D]
- [ ] **#9 — Seed ratio limits** — Auto-pause/remove torrents after reaching a configurable upload:download ratio (global and per-torrent). [Q, D, T]
- [ ] **#10 — Seed time limits** — Auto-pause/remove torrents after seeding for a configurable duration. [Q, D, T]
- [ ] **#11 — Queue management** — Configurable limits on concurrent active downloads/uploads with automatic queuing of excess torrents. [Q, D, T]
- [ ] **#12 — Rarest-first piece selection** — Explicit rarest-first algorithm for optimal swarm health. Current selection is file-priority-based, not rarity-based. [Q, D, T]
- [ ] **#13 — Sequential download mode** — Download pieces in order for preview/playback during download (streaming already works via priority pieces, but no explicit sequential toggle). [Q, D, T]

## File Management

- [ ] **#14 — File priorities (high / normal / low / skip)** — Per-file priority levels within a torrent beyond binary include/exclude. [Q, D, T]
- [ ] **#15 — File preallocation** — `fallocate` / sparse / full preallocation modes to reduce fragmentation. [Q, D, T]
- [ ] **#16 — Move completed downloads** — Automatically move finished torrents from an incomplete directory to a final location. [Q, D, T]
- [ ] **#17 — Rename files and folders** — Rename individual files or the torrent folder via UI/API. [Q, D, T]
- [ ] **#18 — Move torrent data to new location** — Relocate torrent data to a different directory without re-downloading. [Q, D, T]
- [ ] **#19 — Torrent creation** — Create .torrent files (v1, v2, hybrid) from local data, with piece size selection and private flag. [Q, D, T]

## Network

- [ ] **#20 — NAT-PMP port mapping** — Complement existing UPnP with NAT-PMP/PCP for Apple routers and others. [Q, D, T]
- [ ] **#21 — SOCKS4 proxy support** — Currently only SOCKS5; add SOCKS4 for legacy proxy setups. [Q, D, T]
- [ ] **#22 — HTTP/HTTPS proxy support** — HTTP CONNECT proxy for tracker and peer connections. [Q, D, T]
- [ ] **#23 — Per-torrent connection limits** — Limit peers on a per-torrent basis (currently only global `--peer-limit`). [Q, D, T]
- [ ] **#24 — Global connection limit** — Hard cap on total connections across all torrents. [Q, D, T]

## RSS & Automation

- [ ] **#25 — Built-in RSS reader** — Subscribe to RSS/Atom feeds for torrent sites. [Q]
- [ ] **#26 — RSS auto-download rules** — Regex-based filtering rules to automatically download matching torrents from RSS feeds, with per-rule category assignment. [Q, D via YaRSS2]
- [ ] **#27 — Script on torrent completion** — Execute an external program/script when a torrent finishes downloading, with torrent metadata as environment variables. [Q, D, T]
- [ ] **#28 — Script on torrent added** — Execute a script when a new torrent is added. [D]

## Search

- [ ] **#29 — Built-in search engine** — Integrated torrent search across multiple sites with installable Python plugins. [Q]

## Labeling & Organization

- [ ] **#30 — Tags (multiple per torrent)** — Flat labels/tags that can be applied multiple per torrent for flexible organization. [Q, D]
- [ ] **#31 — Hierarchical categories** — Nested category trees with associated save paths. Currently categories are flat (qBittorrent API compat). [Q]
- [ ] **#32 — Filter sidebar** — UI filter panel by status, category, tag, tracker. [Q, D, T]

## Security

- [ ] **#33 — Blocklist format support** — Support PeerGuardian (P2B), DAT, eMule, SafePeer formats in addition to current newline-delimited format. [Q, D, T]
- [ ] **#34 — Automatic blocklist updates** — Periodic refresh of blocklist from URL on a configurable schedule. [Q, D, T]
- [ ] **#35 — RPC IP whitelist** — Restrict API/WebUI access to specific IP ranges beyond auth. [T]
- [ ] **#36 — HTTPS for Web UI** — TLS termination for the built-in HTTP API/Web UI. [Q, D, T]
- [ ] **#37 — Multi-user support** — Multiple user accounts with separate permissions and torrent lists. [D]

## Web UI & UX

- [ ] **#38 — Desktop notifications** — System notifications on torrent complete, errors, etc. (desktop app via Tauri could support this). [Q, D, T]
- [ ] **#39 — System tray icon** — Minimize to system tray on desktop platforms. [Q, D, T]
- [ ] **#40 — Torrent detail view** — Detailed view showing peers, trackers, files, pieces, and transfer graph per torrent. [Q, D, T]
- [ ] **#41 — Tracker management in UI** — Add, edit, remove trackers for active torrents through the UI. [Q, D, T]
- [ ] **#42 — Peer list in UI** — Show connected peers with flags, client, speed, progress per torrent. [Q, D, T]
- [ ] **#43 — Speed graph** — Real-time upload/download speed chart. [Q, D, T]
- [ ] **#44 — Transfer statistics persistence** — Track cumulative upload/download totals across sessions. [Q, D, T]
- [ ] **#45 — Localization / i18n** — Multi-language UI support. [Q, D, T]

## OS Integration

- [ ] **#46 — Daemon mode** — Detach and run as a background daemon (systemd service file exists but no built-in daemonization). [Q, D, T]
- [ ] **#47 — Email notifications** — Send email on torrent events (complete, error). [D]

## Import & Migration

- [ ] **#48 — Import from other clients** — Import torrent list, resume data, and settings from qBittorrent, Deluge, or Transmission.
- [ ] **#49 — Backup and restore** — Export/import full session state for migration between machines.

## Performance & Storage

- [ ] **#50 — Process-level memory ceiling** — Hard cap on memory usage beyond peer limits; graceful degradation under memory pressure.
- [ ] **#51 — DHT cache disk I/O reduction** — Reduce frequency/size of DHT state dumps.
- [ ] **#52 — Settings hot-reload** — Apply configuration changes without restart.

## Platform

- [ ] **#53 — Windows file locking** — Handle locked .exe files during download on Windows (#369).
- [ ] **#54 — Read-only file support** — Skip writing to files marked read-only (#136).

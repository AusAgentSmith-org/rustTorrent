# rustTorrent

A modern, self-hostable BitTorrent client written in Rust — a single small binary with a
clean web UI, a full HTTP API, and qBittorrent-compatible endpoints for the tools you
already run.

> **Alpha software.** rustTorrent is under active development. Expect bugs, breaking
> changes, and incomplete features. [Report issues](https://repo.indexarr.net/indexarr/rustTorrent/issues).

**Website:** [rusttorrent.dev](https://rusttorrent.dev/) ·
**Live demo:** [rusttorrent.dev/demo](https://rusttorrent.dev/demo/) ·
**Discord:** [discord.gg/pu6chSqpnJ](https://discord.gg/pu6chSqpnJ)

## Features

- **Web UI** — responsive React + TypeScript interface with a compact table view, detail
  panes, and dark mode. Works from any browser, desktop or mobile.
- **HTTP API** — everything the UI does is an API call: add torrents, query state, select
  files, stream content. Swagger documentation included.
- **qBittorrent-compatible API** — speaks the qBittorrent WebUI protocol, so Sonarr,
  Radarr, and other *arr applications connect without adapters.
- **Full peer discovery** — DHT, HTTP and UDP trackers, peer exchange, local service
  discovery, and UPnP port forwarding. Magnet links resolve metadata straight from the
  swarm.
- **Streaming** — play media files directly from incomplete torrents over HTTP.
- **RSS automation** — subscribe to feeds with filter rules and download new releases
  automatically.
- **Indexarr integration** — browse and search the [Indexarr](https://indexarr.net/)
  torrent index from inside the web UI.
- **Docker ready** — multi-stage build producing a minimal scratch-based image.
- **Desktop app** — native Tauri application for Windows, macOS, and Linux with system
  tray integration.
- **Memory safe** — written entirely in Rust; the bencode, DHT, and peer-protocol parsers
  are additionally fuzz-tested.

## Quick start

### Docker

```bash
docker run -d --name rusttorrent \
  -p 3030:3030 -p 4240:4240 \
  -v ~/downloads:/downloads \
  ghcr.io/ausagentsmith-org/rusttorrent:alpha
```

Or build from the repository:

```bash
git clone https://repo.indexarr.net/indexarr/rustTorrent.git
cd rustTorrent
docker compose up --build -d
```

**Ports:** `3030` (web UI + API), `4240` (BitTorrent TCP + uTP).

### Prebuilt binaries

Static binaries for Linux and Windows, with SHA256 checksums, are published at
[dl.rusttorrent.dev/latest](https://dl.rusttorrent.dev/latest/):

```bash
curl -LO https://dl.rusttorrent.dev/latest/rtbit-linux-x86_64
chmod +x rtbit-linux-x86_64
./rtbit-linux-x86_64 server start ~/Downloads
```

The web UI comes up at [http://localhost:3030](http://localhost:3030).

### From source

```bash
git clone https://repo.indexarr.net/indexarr/rustTorrent.git
cd rustTorrent
cargo build --release
./target/release/rtbit server start ~/Downloads
```

Requires a recent stable Rust (CI builds with 1.95) and Node.js/npm for the bundled web
UI. See [docs/TESTING.md](docs/TESTING.md) for running the test suites.

## Configuration

Most behaviour is configurable from the web UI's settings dialog or via CLI flags
(`rtbit --help`). Selected environment variables:

| Variable | Purpose |
|----------|---------|
| `RTBIT_INDEXARR_ENABLED` | Enable the Indexarr browse/search integration (`true`/`1`) |
| `RTBIT_INDEXARR_URL` | Base URL of your Indexarr instance |
| `RTBIT_INDEXARR_API_KEY` | API key, injected server-side (never sent to the browser) |

See [documentation/IndexarrRustTorrentIntegration.md](documentation/IndexarrRustTorrentIntegration.md)
for the full integration guide.

## Repository layout

| Path | Contents |
|------|----------|
| `crates/rtbit` | CLI binary and server entry point |
| `crates/librtbit` | Core session, torrent state machine, storage, HTTP API, web UI |
| `crates/librtbit-*` | Protocol building blocks: bencode, DHT, peer protocol, trackers, UPnP, LSD |
| `desktop/` | Tauri desktop application |
| `fuzz/` | Fuzz targets for the protocol parsers |
| `docs/` | Architecture notes, testing guide, fork provenance |

## Attribution

rustTorrent is a permanent hard fork of [rqbit](https://github.com/ikatson/rqbit) by
**Igor Katson**, licensed under Apache 2.0. The original copyright and license notices
are preserved; see [docs/FORK_PROVENANCE.md](docs/FORK_PROVENANCE.md) for details of what
was imported and when. We're grateful for Igor's work — rustTorrent builds on that
foundation.

## License

Apache 2.0 — see [LICENSE](LICENSE).

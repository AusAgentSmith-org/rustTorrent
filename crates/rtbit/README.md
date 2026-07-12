# rustTorrent

A modern BitTorrent client written in Rust. Fast, lightweight, and built for self-hosters.

In controlled local throughput benchmarks, rustTorrent reached up to **15x the throughput** of qBittorrent and exceeded **16 Gbps**. These are LAN/disk-path results, not claims about public-swarm performance. Desktop app, clean web UI, full HTTP API, and runs anywhere Docker does.

**Website:** [rusttorrent.dev](https://rusttorrent.dev/) | **Live Demo:** [rusttorrent.dev/demo](https://rusttorrent.dev/demo/)

## Features

- **High local throughput** -- Up to 15x qBittorrent throughput in the documented LAN/disk benchmarks, peaking above 16 Gbps with Tokio-based async I/O
- **Web UI** -- Clean, responsive React + TypeScript interface. Manage torrents from any browser, desktop or mobile. Dark mode included
- **Desktop App** -- Native application for Windows, macOS, and Linux powered by Tauri. System tray integration, lightweight, and auto-updates
- **HTTP API** -- Full REST API with Swagger documentation. Add torrents, check status, stream files, and manage everything programmatically
- **Docker Ready** -- Official multi-stage Docker image. Minimal scratch-based container with only the binary. One command to deploy
- **DHT & Trackers** -- DHT including BEP-5 peer-wire port exchange, HTTP and UDP announces, peer exchange, and UPnP port forwarding
- **Magnet Links** -- Add torrents via magnet links or .torrent files. Metadata is resolved automatically from the swarm
- **File Streaming** -- Stream media files directly from incomplete torrents via the HTTP API. Perfect for watching videos while they download
- **Indexarr Integration** -- Browse and search the [Indexarr](https://indexarr.net/) torrent index directly from the web UI
- **Memory Safe** -- Rust's ownership model ensures no buffer overflows, data races, or use-after-free bugs
- **Arr Stack Compatible** -- Works with Sonarr, Radarr, Prowlarr, and other *arr applications

## Performance

Benchmarked head-to-head against qBittorrent across six real-world scenarios:

| Scenario | Result |
|----------|--------|
| 8 GB, 1 file, 3 peers | ~1x (even) |
| 16 GB, 1 file, 3 peers | 2.5x faster |
| 8 GB, 10 files, 3 peers | 2.6x faster |
| 8 GB, 100 files, 3 peers | **14.7x faster** |
| 8 GB, 1 file, 100 peers | 2.6x faster |
| 8 GB, 1 file, 500 peers | 2.4x faster |

All benchmarks run in Docker containers on identical hardware with direct peer-to-peer transfers. Full methodology and scripts available in the `benchv2/` directory.

## Getting Started

### Desktop

Download the installer for your platform from [GitHub Releases](https://github.com/AusAgentSmith-org/rustTorrent/releases/tag/desktop-latest):

- **Windows** -- [.exe](https://github.com/AusAgentSmith-org/rustTorrent/releases/download/desktop-latest/rtbit-desktop_0.0.1_x64-setup.exe) | [.msi](https://github.com/AusAgentSmith-org/rustTorrent/releases/download/desktop-latest/rtbit-desktop_0.0.1_x64_en-US.msi)
- **macOS** -- [Intel .dmg](https://github.com/AusAgentSmith-org/rustTorrent/releases/download/desktop-latest/rtbit-desktop_0.0.1_x64.dmg) | [Apple Silicon .dmg](https://github.com/AusAgentSmith-org/rustTorrent/releases/download/desktop-latest/rtbit-desktop_0.0.1_aarch64.dmg)
- **Linux** -- [.deb](https://github.com/AusAgentSmith-org/rustTorrent/releases/download/desktop-latest/rtbit-desktop_0.0.1_amd64.deb) | [.rpm](https://github.com/AusAgentSmith-org/rustTorrent/releases/download/desktop-latest/rtbit-desktop-0.0.1-1.x86_64.rpm) | [.AppImage](https://github.com/AusAgentSmith-org/rustTorrent/releases/download/desktop-latest/rtbit-desktop_0.0.1_amd64.AppImage)

### Docker

```bash
git clone https://github.com/AusAgentSmith-org/rustTorrent.git
cd rustTorrent
docker compose up --build -d
```

**Ports:** 3030 (Web UI + API), 4240 (BitTorrent TCP + uTP)

### From Source

```bash
git clone https://github.com/AusAgentSmith-org/rustTorrent.git
cd rustTorrent
cargo build --release --features webui
./target/release/rtbit server start
```

Requires Rust 1.75+ and npm (for the webui feature).

Web UI available at [http://localhost:3030](http://localhost:3030)

## Architecture

| Layer | Components |
|-------|------------|
| **Core Library** | librtbit, Session Manager, Torrent State Machine, Pluggable Storage |
| **Networking** | DHT (BEP-5), Peer Wire Protocol, HTTP/UDP Trackers, UPnP |
| **Frontend** | React 18, TypeScript, Tailwind CSS, Zustand, Vite |
| **Infrastructure** | Tokio Async Runtime, Axum HTTP Server, Docker (scratch), Multi-stage Build |

## Attribution

rustTorrent is a fork of [rqbit](https://github.com/ikatson/rqbit), originally created by **Igor Katson**. The original project is licensed under the **Apache License 2.0**.

We are grateful for Igor's work in building a high-quality BitTorrent client in Rust. rustTorrent builds upon that foundation with additional features and modifications.

## License

Apache 2.0 -- See [LICENSE](LICENSE).

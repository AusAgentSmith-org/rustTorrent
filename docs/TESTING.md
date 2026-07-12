# Testing

rustTorrent uses three complementary test layers.

## Rust unit and integration tests

Run the workspace suite with:

```sh
cargo test --workspace --exclude rtbit-desktop --no-default-features --features default-tls
```

The `librtbit` suite includes a deterministic in-process BEP 15 UDP tracker. It
tests connect, announce, and scrape packets through the production tracker
client. A full swarm test then creates a torrent, announces a seed and leech to
that tracker, verifies peer discovery without preconfigured peers, transfers the
payload over the production peer protocol, and compares the downloaded bytes.
No public tracker or internet peer is required.

Authentication tests cover token issuance and rotation, refresh revocation,
credential validation, on-disk reload, malformed credential files, and private
credential-file permissions. qBittorrent compatibility tests cover its special
category filter values in addition to numeric boundary behavior.

## Web UI unit and build checks

```sh
cd crates/librtbit/webui
npm ci
npm test
npm run lint
npm run build
```

## Browser GUI tests

Install Chromium once, then run the Playwright suite:

```sh
cd crates/librtbit/webui
npx playwright install chromium
npm run test:e2e
```

The suite starts the deterministic mock UI automatically and checks loading and
virtualization of 1,000 torrents, searching and status filters, selection and
pause actions, configuration, and the responsive mobile card layout. Use
`npm run test:e2e:headed` for an interactive browser.

Woodpecker runs all three layers on pushes, pull requests, tags, and manual
builds. Failed GUI runs retain a screenshot and Playwright trace in the job
workspace for diagnosis.

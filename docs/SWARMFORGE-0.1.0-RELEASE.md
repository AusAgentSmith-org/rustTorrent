# SwarmForge 0.1.0 release record

Release preparation date: 2026-07-29

This is the immutable release and rollback record for the first coordinated
`swarmforge-*` package family. It records the old-family boundary before any
new crates.io publication and will be extended with the published package
checksums after the registry accepts version 0.1.0.

## Compatibility contract

The first public family release uses one coordinated version, `0.1.0`, for all
12 packages. Cargo package identities change to `swarmforge-*`; Rust library
names and import paths remain compatible. Consumers may continue to use
`librtbit::Session`, `librtbit::Api`, `librtbit_core::*`, and the existing
support-crate import names by declaring Cargo dependency aliases.

Consumers must migrate every member they use atomically. Mixing old
`librtbit-*` and new `swarmforge-*` package identities can duplicate shared
types and break trait coherence.

The historical repositories and existing packages are retained, unmodified,
through the rustTorrent production soak and until every downstream consumer
has completed its separately reviewed migration. There is no scheduled removal
or yank date. This repository remains named rustTorrent; the `rtbit` binary,
runtime names, environment variables, persisted state, and HTTP API are not
renamed by this release.

## Old-family monorepo boundary

The exact pre-migration rustTorrent source is tagged
`swarmforge-old-family-baseline-20260729`.

| Field | Value |
| --- | --- |
| Commit | `195ef411135e10f63d5ef370208c0fdb83772c37` |
| Git tree | `7293bfa72ed85796243bde4f7c761ddd5350f19a` |
| `git archive --format=tar` SHA-256 | `2b16f61b697a630c489c2fda5039d7b0972250e19944b33aa9787ccabeaf84f4` |
| Canonical repository ID | `1206343877` |
| Canonical source | `https://github.com/TheDancingDeveloper-org/rustTorrent` |

The workspace package versions at that boundary were:

| Old package | Version |
| --- | --- |
| `librtbit` | `0.1.6` |
| `librtbit-core` | `0.1.5` |
| `librtbit-bencode` | `0.1.5` |
| `librtbit-buffers` | `0.1.2` |
| `librtbit-clone-to-owned` | `0.1.1` |
| `librtbit-dht` | `0.1.4` |
| `librtbit-lsd` | `0.1.2` |
| `librtbit-peer-protocol` | `0.1.4` |
| `librtbit-sha1-wrapper` | `0.1.2` |
| `librtbit-tracker-comms` | `0.1.6` |
| `librtbit-upnp` | `0.1.2` |
| `librtbit-upnp-serve` | `0.1.2` |

## Historical standalone source repositories

All repositories below were verified on 2026-07-29 with default branch `main`
and `archived=false`. They remain rollback sources.

| Repository | GitHub repository ID | HEAD commit |
| --- | ---: | --- |
| `librtbit` | `1202603883` | `49675c1d79a32aa90ef3806bed4a888bc56d2d23` |
| `librtbit-bencode` | `1202591019` | `255c93423c6fc86da5d9227457da28c8c1f3fdd6` |
| `librtbit-buffers` | `1202590644` | `60fde393b379face7c0acfe7f654a1a5ae3f3731` |
| `librtbit-clone-to-owned` | `1202588618` | `6944f73c91e9a4cc3be7694f864c5c06012066fc` |
| `librtbit-core` | `1202592291` | `486540bb77448ae50df5e2ca846aa0caeef5314e` |
| `librtbit-dht` | `1202595252` | `05f1aa116e767a8020cd459cd43fdd5907c55ada` |
| `librtbit-lsd` | `1202594813` | `79a7b6d4a1dedc0631bbfab43faa576f71ba7e4a` |
| `librtbit-peer-protocol` | `1202594486` | `f91f07946002b9348d7d8c9378cf3b5206f77160` |
| `librtbit-sha1-wrapper` | `1202588836` | `26062b9c6d4e73ad8b061f366d01056d401e467b` |
| `librtbit-tracker-comms` | `1202599388` | `96580fb94839b71fd2ebbfb11dded5d93c53e808` |
| `librtbit-upnp` | `1202589098` | `ffda20d442aa8deee65e5bedf65139ed7de1bb32` |
| `librtbit-upnp-serve` | `1202601459` | `5e1a16554e968ac1bae6b8050f2c38c02db53251` |

## Existing public old-package checksums

These are the newest public crates.io entries observed immediately before the
SwarmForge release. `UNPUBLISHED` means the old identity had no sparse-index
entry; it does not mean the standalone Git repository is absent.

| Old package | Public version | crates.io checksum |
| --- | --- | --- |
| `librtbit` | UNPUBLISHED | - |
| `librtbit-core` | `0.1.3` | `478c51123ed65a4aa1425d535abba96abaf0990ea9dc82eff4943da253561854` |
| `librtbit-bencode` | `0.1.3` | `95e1f155f053c5a2aee47add5141a1c76dbed5f8c511ced3503f79ba1fbf668f` |
| `librtbit-buffers` | `0.1.1` | `29b8cebbaebfe945a5bcc23621c77037c8c9a451b84c9cdd0dadb80cc16414a3` |
| `librtbit-clone-to-owned` | `0.1.1` | `25ef4d419efb961a6ebc8c5122f835ccfe8b918f16fa83b7001a051039889d9b` |
| `librtbit-dht` | `0.1.1` | `24c85ed4287bf4e99868f4acd646329a12db567ae2c63918421457494e578d23` |
| `librtbit-lsd` | UNPUBLISHED | - |
| `librtbit-peer-protocol` | UNPUBLISHED | - |
| `librtbit-sha1-wrapper` | `0.1.1` | `7de78d6fc1f1523fb0416aa64196f2de444102db6026c5ac7a800fb6c4ee5de4` |
| `librtbit-tracker-comms` | UNPUBLISHED | - |
| `librtbit-upnp` | UNPUBLISHED | - |
| `librtbit-upnp-serve` | UNPUBLISHED | - |

## Coordinated publication order

1. `swarmforge-clone-to-owned`
2. `swarmforge-buffers`
3. `swarmforge-sha1-wrapper`
4. `swarmforge-bencode`
5. `swarmforge-core`
6. `swarmforge-peer-protocol`
7. `swarmforge-dht`
8. `swarmforge-lsd`
9. `swarmforge-tracker-comms`
10. `swarmforge-upnp`
11. `swarmforge-upnp-serve`
12. `swarmforge`

`scripts/publish.sh --execute` dry-runs each package immediately before its
upload and waits until crates.io's sparse index exposes version 0.1.0 before
moving to the next dependency level. A pre-existing package name or version is
a hard failure, not a successful rerun.

## Published SwarmForge artifacts

This section is intentionally pending until crates.io has accepted all 12
immutable packages. Record the exact source commit, source tag, package URLs,
and sparse-index checksums here immediately after publication.

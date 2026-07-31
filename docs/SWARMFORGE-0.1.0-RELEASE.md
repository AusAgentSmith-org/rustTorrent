# SwarmForge 0.1.0 release record

Release preparation date: 2026-07-29

This is the immutable release and rollback record for the first coordinated
`swarmforge-*` package family. It records the old-family boundary, published
package checksums, source provenance, and independent consumer verification.

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
a hard failure in normal mode. `--execute --resume` exists only for recovery
from a partially accepted coordinated publication: it verifies and skips an
exact, non-yanked 0.1.0 and rejects every other registry state.

## Published SwarmForge artifacts

All 12 immutable packages were accepted by crates.io on 2026-07-29 and were
independently verified in the authoritative sparse index as exactly one,
non-yanked `0.1.0` entry per package.

The first seven packages were published from the original coordinated release
commit and tag. An isolated-package dry-run then exposed a missing default SHA
backend in `swarmforge-lsd`; no LSD artifact had been uploaded. The correction
made the LSD feature contract explicit, propagated the root SHA feature, and
made the publisher safely resumable. The final five packages were published
from that correction commit and tag.

| Source set | Commit | Tag | Pull request |
| --- | --- | --- | --- |
| Initial seven packages | `29901dba6b8dc10b50fa78639e41daf3f38d429c` | `swarmforge-v0.1.0` | `#21` |
| Corrected final five packages | `d5df464bcc73e557aff95b7e7a7745d7e4626d1f` | `swarmforge-v0.1.0-lsd-fix` | `#22` |

| Package | Source set | crates.io checksum |
| --- | --- | --- |
| [`swarmforge-clone-to-owned`](https://crates.io/crates/swarmforge-clone-to-owned/0.1.0) | Initial | `2bb3b8f73c82f8996e0ace7e18f6122b679121e6e2c2375b12785a8d4b6ae3de` |
| [`swarmforge-buffers`](https://crates.io/crates/swarmforge-buffers/0.1.0) | Initial | `3d580701883234fe6f873c0996e38feb829635ef970bbf341eb44ba148a27ed2` |
| [`swarmforge-sha1-wrapper`](https://crates.io/crates/swarmforge-sha1-wrapper/0.1.0) | Initial | `433e984d736e80f61a4171482657ec6bd0108b0803a973bd84c712e571e0481b` |
| [`swarmforge-bencode`](https://crates.io/crates/swarmforge-bencode/0.1.0) | Initial | `62025700dfaaaaa83979d2660a6fc14370ed273990cce0dbc76b0782a22fdb05` |
| [`swarmforge-core`](https://crates.io/crates/swarmforge-core/0.1.0) | Initial | `f832d9e4a4b8da07731824ce53ccec3b1efa26007d28a9a20bda0c2cd9a190a5` |
| [`swarmforge-peer-protocol`](https://crates.io/crates/swarmforge-peer-protocol/0.1.0) | Initial | `f5bb477c07172aeeebe5c363472e210d0766073b40ef39475aa3ac5e486b7c94` |
| [`swarmforge-dht`](https://crates.io/crates/swarmforge-dht/0.1.0) | Initial | `b906ebbdf427da92ed396e1467e73bd773c95065f62f501b422268d8dee9a957` |
| [`swarmforge-lsd`](https://crates.io/crates/swarmforge-lsd/0.1.0) | Corrected | `7d97411d5e5bf18353e3ca6fc22f3dc42659b7217bf53aec70f6d6f24cf7978e` |
| [`swarmforge-tracker-comms`](https://crates.io/crates/swarmforge-tracker-comms/0.1.0) | Corrected | `e2c1a9c97cd297611004f9c56ea00c2587492bfd409aac1e55162abad27da659` |
| [`swarmforge-upnp`](https://crates.io/crates/swarmforge-upnp/0.1.0) | Corrected | `74d398b4babe325834bd32ea02c3fd97dacda0e3f0b365ac484fe71f62ea3825` |
| [`swarmforge-upnp-serve`](https://crates.io/crates/swarmforge-upnp-serve/0.1.0) | Corrected | `2b438743bf8cbc23f6ef9aece56c1c4d1ab25cf943dda734d91f09bd48c2cb61` |
| [`swarmforge`](https://crates.io/crates/swarmforge/0.1.0) | Corrected | `91806d3fedb13ffdfd86e903d62c6e0a46921c1b080ca0a105e25887fcaff7e2` |

## Verification evidence

The protected-branch checks passed for both release pull requests. The initial
release CI run was `30418493414`; its post-merge CI and container publication
runs were `30419119860` and `30419119856`. The LSD correction CI run was
`30420931263`; its post-merge CI and container publication runs were
`30421472136` and `30421472180`. Required policy checks passed for both changes
and both tags.

After all packages were visible, a fresh anonymous checkout of the correction
tag was prepared with an empty `CARGO_HOME`, no registry credentials, no
monorepo path overrides, and a newly generated lockfile. Cargo metadata proved
that the `rtbit` application's direct SwarmForge dependencies resolved from
crates.io. The following consumer gates then passed with `spin 0.9.9` selected
by the clean lockfile:

```text
cargo check --locked -p rtbit --no-default-features --features default-tls
cargo test --locked -p rtbit --no-default-features --features default-tls
cargo clippy --locked -p rtbit --no-default-features --features default-tls -- -D warnings
```

The consumer test result was 2 passed, 0 failed. This clean checkout did not
use Forgejo packages, credentials, or locally patched SwarmForge crates.

## Downstream consumer checkpoint: NGMS

NGMS became the first independently migrated external consumer of the complete
SwarmForge 0.1.0 family on 2026-07-30/31 UTC. Canonical GitHub PR
[`TheDancingDeveloper-org/NGMS#3`](https://github.com/TheDancingDeveloper-org/NGMS/pull/3)
was squash-merged as source-migration commit
`dbfacacac873c4f7faeaf8576465620a580587ba`. It atomically replaced all 12
active Forgejo Cargo dependencies with exact crates.io `swarmforge-* = 0.1.0`
aliases, regenerated the lockfile, retained the old vendored tree as an
explicitly inactive rollback snapshot, and added a public-dependency boundary
check that rejects private Git, Forgejo, and path fallback.

Initial main run `30591519527` passed the dependency, Rust, UI, Playwright, and
GHCR publication gates and published immutable image
`ghcr.io/thedancingdeveloper-org/ngms:sha-dbfacacac873c4f7faeaf8576465620a580587ba`
at public digest
`sha256:d70574a4293805bfe1a8752aaa1df9942881579a664cd7a8e9c4080d7fa97d7a`.
The run was cancelled after artifact verification because an optional BuildKit
GHA cache export wedged after the image push.

Follow-up PR
[`TheDancingDeveloper-org/NGMS#21`](https://github.com/TheDancingDeveloper-org/NGMS/pull/21)
retained cache reads, removed the non-essential cache write, and merged as
current NGMS `main` commit `e1ab4b2296bbf9280e107768fb991f8de85298cd`.
[Main run `30595845085`](https://github.com/TheDancingDeveloper-org/NGMS/actions/runs/30595845085)
then completed successfully across the public dependency check, formatting,
locked all-feature workspace check/test/clippy, both UI builds, Playwright E2E,
and container publication. Its immutable tag
`ghcr.io/thedancingdeveloper-org/ngms:sha-e1ab4b2296bbf9280e107768fb991f8de85298cd`
and `latest` resolve to public, canonical-repository manifest digest
`sha256:7d929bf55a742b661b4db8cf7a980569e4ca54d902f7a9053a1bbb88f9326568`.

This checkpoint proves crates.io consumption and canonical source/build
publication only. It did not deploy NGMS, alter an ops or Komodo stack, change
ARR or seed state, migrate another consumer, retire Forgejo rollback surfaces,
or rotate/remediate credentials. Those remain separately reviewed gates.

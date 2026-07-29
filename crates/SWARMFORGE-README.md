# SwarmForge

SwarmForge is the reusable BitTorrent engine and protocol-crate family that
powers [rustTorrent](https://github.com/TheDancingDeveloper-org/rustTorrent).
The public packages are released together from the rustTorrent monorepo.

The first coordinated release changes Cargo package identities from the
historical `librtbit-*` names to `swarmforge-*`. Rust library names remain
compatible, so existing imports such as `librtbit::Session`,
`librtbit_core::Id20`, and `librtbit_bencode` continue to work when dependencies
use an alias:

```toml
librtbit = { package = "swarmforge", version = "0.1" }
librtbit-core = { package = "swarmforge-core", version = "0.1" }
bencode = { package = "swarmforge-bencode", version = "0.1" }
```

Consumers must migrate every SwarmForge family member in one dependency graph
atomically. Mixing old and new package identities can duplicate shared types and
break trait coherence. The historical repositories and packages remain available
as rollback sources while downstream consumers migrate.

SwarmForge is derived from [rqbit](https://github.com/ikatson/rqbit) by Igor
Katson. Copyright 2021 Igor Katson. The original Apache-2.0 license applies; see
the rustTorrent repository's [license](https://github.com/TheDancingDeveloper-org/rustTorrent/blob/main/LICENSE)
and
[fork provenance](https://github.com/TheDancingDeveloper-org/rustTorrent/blob/main/docs/FORK_PROVENANCE.md)
for details.

Source, issues, and release history are maintained in the
[rustTorrent repository](https://github.com/TheDancingDeveloper-org/rustTorrent).

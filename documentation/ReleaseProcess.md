# rustTorrent Release Process

GitHub is the source of truth for source code, CI, releases, GHCR, and the
rusttorrent.dev website. The historical Forgejo/Woodpecker path is retired.

## Versioning

The current beta release is `0.1.0-beta.2`.

- Crate version: `crates/rtbit/Cargo.toml`
- Release tag: `v0.1.0-beta.2`
- Docker tag: `ghcr.io/thedancingdeveloper-org/rusttorrent:v0.1.0-beta.2`

## CI Flow

Pushes and tags run the workflows under `.github/workflows/`.

The Rust workflow builds the workspace from GitHub. Local development keeps
`[patch]` sections pointed at sibling crates under `../libs/`.

Main branch pushes publish the public GHCR image as `dev` and an immutable
`sha-<commit>` tag.

Release tags build:

- `rtbit-<tag>-linux-x86_64`
- `rtbit-<tag>-windows-x86_64.exe`
- `rtbit-<tag>-amd64.deb`
- `SHA256SUMS-<tag>.txt`
- Docker manifest for `linux/amd64` and `linux/arm64`

Tag releases publish artifacts to the download host and create the canonical
GitHub release. The container workflow publishes the multi-arch GHCR image.

Release notes are checked in as `RELEASE_NOTES_<tag>.md`; the GitHub release
workflow uses that file verbatim and uploads the binaries and checksums.

GitHub publishes every tagged build as a full release (`prerelease: false`),
including tags whose semantic version contains `alpha`, `beta`, or `rc`.
The Docker image must keep `org.opencontainers.image.source` set to
`https://github.com/TheDancingDeveloper-org/rustTorrent`. GitHub uses that OCI source
metadata to associate the GHCR container package with the public repository so
it appears on `https://github.com/TheDancingDeveloper-org/rustTorrent/packages`.

## Auth and Permission Notes

Use the workspace `docs/AUTH-VALIDATION.md` before touching Infisical, GitHub,
GHCR, or Komodo state.

Known permission failure modes:

- If the auth check reports a missing `INFISICAL_CLIENT_ID`, the running
  MyDevEnv2 container has not picked up the Komodo stack env values. Redeploy
  `prod-mydevenv2` from outside the active session; restarting the current
  container terminates the agent session.
- Woodpecker API calls must use numeric repo IDs. Owner/name pipeline endpoints
  can return the frontend HTML with HTTP 200 and do no useful work.
- GHCR copy and GitHub release steps use the Woodpecker `gh_release_token`
  secret. If GitHub validation returns `401` or GHCR returns `403`, rotate
  `GITHUB_PAT` / `GH_RELEASE_TOKEN` in Infisical `cicd`, then refresh the
  Woodpecker `gh_release_token` secret before tagging a release. The token
  needs repository release access and GHCR package write access.

## v0.1.0-beta.1 Outcome

Woodpecker pipeline 71 published the Forgejo release and download artifacts on
2026-06-12. The initial GHCR and GitHub release stages failed because the
Woodpecker `gh_release_token` secret no longer had usable GitHub/GHCR access.
The token was rotated into Infisical `cicd/prod` as `GITHUB_PAT` and
`GH_RELEASE_TOKEN`, and the Woodpecker repo secret `gh_release_token` was
updated with `manual`, `push`, and `tag` event access.

- Forgejo release ID: `416`
- GitHub release ID: `338417179`
- Linux x86_64, Windows x86_64, Debian amd64, and SHA256 assets: published to
  Forgejo and GitHub
- Forgejo Docker image: published for `linux/amd64` and `linux/arm64`
- Historical GHCR image: `ghcr.io/ausagentsmith-org/rusttorrent:v0.1.0-beta.1`,
  `latest`, and `beta` published for `linux/amd64` and `linux/arm64`
- GitHub mirror: release tag `v0.1.0-beta.1` dereferences to the release-prep
  commit `6d040d0`; release follow-up documentation is on `main`
- GitHub issues `#11` through `#15`: closed with direct links to the `librtbit`
  implementation commit and the rustTorrent release-integration commit

The PAT used for the repair was provided in chat. Rotate it again after the
release repair is complete, then update Infisical `GITHUB_PAT` /
`GH_RELEASE_TOKEN` and the Woodpecker `gh_release_token` secret with the
replacement value.

Do not print tokens, use `set -x`, or write credentials to repo files.

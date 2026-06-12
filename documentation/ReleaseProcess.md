# rustTorrent Release Process

Forgejo is the source of truth for source code and Woodpecker is the release
runner. GitHub and GHCR are public distribution targets only.

## Versioning

The next beta release is `0.1.0-beta.1`.

- Crate version: `crates/rtbit/Cargo.toml`
- Release tag: `v0.1.0-beta.1`
- Docker tag: `ghcr.io/ausagentsmith-org/rusttorrent:v0.1.0-beta.1`

## CI Flow

Pushes and tags run `.woodpecker.yml`.

The Rust steps strip local `[patch]` sections before building so CI proves the
app can consume the published `librtbit-*` crates from the Forgejo Cargo
registry. Local development keeps `[patch]` sections pointed at sibling crates
under `../libs/`.

Main branch pushes build and push the Forgejo Docker image as:

- `repo.indexarr.net/indexarr/rusttorrent:latest`
- `repo.indexarr.net/indexarr/rusttorrent:<commit-sha>`

Release tags build:

- `rtbit-<tag>-linux-x86_64`
- `rtbit-<tag>-windows-x86_64.exe`
- `rtbit-<tag>-amd64.deb`
- `SHA256SUMS-<tag>.txt`
- Docker manifest for `linux/amd64` and `linux/arm64`

Tag releases publish artifacts to the download host, create a Forgejo release,
push the multi-arch Docker image to Forgejo, copy it to GHCR, then create the
GitHub release.

## Auth and Permission Notes

Use `/home/AusAgentSmith/Working/docs/AUTH-VALIDATION.md` before touching Forgejo,
Woodpecker, Infisical, GitHub, GHCR, or Komodo state.

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
2026-06-12:

- Forgejo release ID: `416`
- Linux x86_64, Windows x86_64, Debian amd64, and SHA256 assets: published
- Forgejo Docker image: published for `linux/amd64` and `linux/arm64`
- GHCR copy: blocked by `403 Forbidden` from the destination bearer-token
  request
- GitHub release: skipped because the preceding GHCR step failed

The public GHCR and GitHub release stages require token rotation before they can
be retried successfully.

Do not print tokens, use `set -x`, or write credentials to repo files.

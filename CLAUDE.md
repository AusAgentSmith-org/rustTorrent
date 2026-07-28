# rustTorrent

BitTorrent client written in Rust with a web UI.

## Repository & credentials (read before pushing or filing issues)

- **Source of truth is GitHub**: `origin` →
  `https://github.com/TheDancingDeveloper-org/rustTorrent.git`. Push code and
  file issues there.
- Do not inspect, fetch, compare, or push the historical Forgejo namesake
  unless the task explicitly concerns rollback or Forgejo deprecation. The
  workspace routing policy is `docs/GITHUB-MIGRATION-ROUTING.md`.
- **Credentials come from Infisical**, not from a logged-in `gh`/git session (there is none).
  This runtime has an injected machine identity (`INFISICAL_CLIENT_ID`/`INFISICAL_CLIENT_SECRET`);
  `mydevenv2-agent-auth check` confirms access. Do not declare yourself "blocked on auth" without
  fetching from Infisical first. Paths and token names are in the workspace-level `AGENTS.md`
  (Service Access table): `GIT_AUTH_TOKEN`/`FORGEJO_TOKEN`/`GITHUB_PAT` live in the `cicd` project,
  env `prod`. Never print token values.

## Codesight

Auto-generated codebase context map: `.codesight/CODESIGHT.md` — routes, schema, components, dependencies, and hot files. Regenerate with `npx codesight`.

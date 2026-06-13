# rustTorrent

BitTorrent client written in Rust with a web UI.

## Repository & credentials (read before pushing or filing issues)

- **Source of truth is Forgejo**: `origin` → `https://repo.indexarr.net/indexarr/rustTorrent.git`.
  Push code and **file issues here** (REST API under `/api/v1/`).
- The `github` remote (`AusAgentSmith-org/rustTorrent`) is a **public mirror only** — website/README,
  GHCR images. Do **not** push private source or file dev issues there.
- **Credentials come from Infisical**, not from a logged-in `gh`/git session (there is none).
  This runtime has an injected machine identity (`INFISICAL_CLIENT_ID`/`INFISICAL_CLIENT_SECRET`);
  `mydevenv2-agent-auth check` confirms access. Do not declare yourself "blocked on auth" without
  fetching from Infisical first. Full paths + token names are in `/home/AusAgentSmith/Working/AGENTS.md`
  (Service Access table): `GIT_AUTH_TOKEN`/`FORGEJO_TOKEN`/`GITHUB_PAT` live in the `cicd` project,
  env `prod`. Never print token values.

## Codesight

Auto-generated codebase context map: `.codesight/CODESIGHT.md` — routes, schema, components, dependencies, and hot files. Regenerate with `npx codesight`.

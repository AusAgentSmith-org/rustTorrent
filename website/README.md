# rusttorrent.dev website

This directory is the canonical source for the static website served at
[rusttorrent.dev](https://rusttorrent.dev). It was imported from the former
`indexarr/rustTorrent_Website` repository as a Git subtree, following the
monorepo layout used by rustnzbd.

## Development

Serve this directory with any static HTTP server, for example:

```sh
python3 -m http.server --directory website 8080
```

The interactive demo is a tracked production build under `website/demo/`.

Run the same structural check used by CI from the repository root:

```sh
ci/tasks/website-check
```

## Deployment

The root `.woodpecker.yml` owns website CI/CD. Every pipeline validates the
site's entry points and local assets. A push to `main` synchronizes
`website/` to `/root/websites/rusttorrent` on the Vultr web host and verifies
the deployed entry points. The standalone repository pipelines are retained
only in the imported Git history and must not be re-enabled.

Application releases and `dl.rusttorrent.dev` remain separate from this static
site deployment.

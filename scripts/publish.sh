#!/usr/bin/env bash
set -euo pipefail

publish_flag="--dry-run"
if [[ "${1:-}" == "--execute" ]]; then
  publish_flag=""
  shift
fi

version="0.1.0"

# Keep this list explicit: Cargo package publication must follow dependency
# order, and directory discovery can publish the binary or a leaf before the
# family it depends on. The old librtbit-* repositories and packages remain
# available as rollback assets; this script publishes only the coordinated
# swarmforge family.
packages=(
  swarmforge-clone-to-owned
  swarmforge-buffers
  swarmforge-sha1-wrapper
  swarmforge-bencode
  swarmforge-core
  swarmforge-peer-protocol
  swarmforge-dht
  swarmforge-lsd
  swarmforge-tracker-comms
  swarmforge-upnp
  swarmforge-upnp-serve
  swarmforge
)

sparse_path() {
  local package="$1"
  case "${#package}" in
    1) printf '1/%s' "$package" ;;
    2) printf '2/%s' "$package" ;;
    3) printf '3/%s/%s' "${package:0:1}" "$package" ;;
    *) printf '%s/%s/%s' "${package:0:2}" "${package:2:2}" "$package" ;;
  esac
}

package_is_visible() {
  local package="$1" body
  body="$(curl --fail --silent --show-error \
    --user-agent 'cargo swarmforge-release' \
    "https://index.crates.io/$(sparse_path "$package")")" || return 1
  jq --exit-status --arg version "$version" \
    'select(.vers == $version and (.yanked | not))' \
    >/dev/null <<<"$body"
}

if [[ -z "$publish_flag" ]]; then
  command -v curl >/dev/null
  command -v jq >/dev/null
  [[ -n "${CARGO_REGISTRY_TOKEN:-}" ]] || {
    echo 'CARGO_REGISTRY_TOKEN is required for --execute' >&2
    exit 1
  }

  # Names are checked as one family immediately before the first upload. Any
  # existing sparse-index entry means ownership changed and publication stops.
  for package in "${packages[@]}"; do
    status="$(curl --silent --output /dev/null --write-out '%{http_code}' \
      --user-agent 'cargo swarmforge-release' \
      "https://index.crates.io/$(sparse_path "$package")")"
    [[ "$status" == "404" ]] || {
      echo "refusing publication: $package sparse-index status is $status, expected 404" >&2
      exit 1
    }
  done
fi

for package in "${packages[@]}"; do
  cargo publish --locked --dry-run -p "$package" "$@"
  [[ -z "$publish_flag" ]] || continue

  cargo publish --locked -p "$package" "$@"
  for _ in {1..60}; do
    package_is_visible "$package" && break
    sleep 2
  done
  package_is_visible "$package" || {
    echo "$package $version was uploaded but did not appear in the sparse index" >&2
    exit 1
  }
done

#!/usr/bin/env bash

set -euo pipefail

forbidden="$(printf '%s%s%s' 'sp' 'roo' 'ty')"
failures=0

fail() {
  printf 'migration policy: %s\n' "$1" >&2
  failures=$((failures + 1))
}

if git grep -aiqi -- "$forbidden" HEAD --; then
  git grep -aini -- "$forbidden" HEAD -- >&2 || true
  fail 'forbidden legacy identifier found in tracked content'
fi

while IFS= read -r -d '' path; do
  if [[ "${path,,}" == *"$forbidden"* ]]; then
    printf '%s\n' "$path" >&2
    fail 'forbidden legacy identifier found in tracked path'
  fi
done < <(git ls-files -z)

if ! python3 - <<'PY'
import pathlib
import re
import sys

failed = False
for path in sorted(pathlib.Path('.github/workflows').glob('*.y*ml')):
    lines = path.read_text(encoding='utf-8').splitlines()

    def indent(line: str) -> int:
        return len(line) - len(line.lstrip(' '))

    def values(index: int) -> list[str]:
        line = lines[index]
        base = indent(line)
        value = line.split(':', 1)[1].split('#', 1)[0].strip()
        if value:
            return [part.strip().strip('"\'') for part in value.strip('[]').split(',') if part.strip()]
        result = []
        for child in lines[index + 1:]:
            stripped = child.strip()
            if not stripped or stripped.startswith('#'):
                continue
            if indent(child) <= base:
                break
            match = re.match(r'^-\s*([^#]+)', stripped)
            if match:
                result.append(match.group(1).strip().strip('"\''))
        return result

    for number, line in enumerate(lines):
        if not re.match(r'^\s*runs-on\s*:', line, re.I):
            continue
        selected = values(number)
        if not any(value.lower() == 'self-hosted' for value in selected):
            print(f'{path}:{number + 1}: runner selection is not explicitly self-hosted', file=sys.stderr)
            failed = True
        if any('${{' in value for value in selected):
            print(f'{path}:{number + 1}: dynamic runner selection requires explicit review', file=sys.stderr)
            failed = True

sys.exit(1 if failed else 0)
PY
then
  fail 'workflow selects an unapproved runner'
fi

if [[ -n "${GITHUB_BASE_REF:-}" ]]; then
  git fetch --no-tags origin "$GITHUB_BASE_REF" >/dev/null 2>&1 || true
  base="origin/$GITHUB_BASE_REF"
  commits="$(git rev-list --reverse "$base..HEAD" 2>/dev/null || git rev-list --reverse HEAD~1..HEAD 2>/dev/null || git rev-list --reverse HEAD)"
elif [[ "${GITHUB_EVENT_NAME:-}" == push && "${GITHUB_BEFORE:-}" != 0000000000000000000000000000000000000000 ]]; then
  commits="$(git rev-list --reverse "${GITHUB_BEFORE:-}..HEAD" 2>/dev/null || git rev-list --reverse HEAD~1..HEAD 2>/dev/null || git rev-list --reverse HEAD)"
else
  commits="$(git rev-list --reverse HEAD~1..HEAD 2>/dev/null || git rev-list --reverse HEAD)"
fi

while IFS= read -r commit; do
  [[ -n "$commit" ]] || continue
  metadata="$(git show -s --format='%an%n%ae%n%cn%n%ce%n%B' "$commit")"
  if grep -qi -- "$forbidden" <<<"$metadata"; then
    printf '%s\n' "$commit" >&2
    fail 'forbidden legacy identifier found in commit metadata'
  fi
done <<<"$commits"

((failures == 0)) || exit 1
printf 'migration policy passed\n'

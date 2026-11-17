#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
repo="${PEEP_GITHUB_REPO:-manasv/peep}"
run="${1:-}"
if [[ -z "$run" ]]; then
  run="$(gh run list --repo "$repo" --branch "$(git branch --show-current)" \
    --workflow build.yml --limit 1 --json databaseId --jq '.[0].databaseId // empty')"
fi
if [[ -z "$run" ]]; then echo 'No CI run found for this branch.' >&2; exit 1; fi
gh run view "$run" --repo "$repo"
jobs="$(gh run view "$run" --repo "$repo" --json jobs \
  --jq '.jobs[] | select(.conclusion == "failure") | .databaseId')"
while IFS= read -r job; do
  if [[ -n "$job" ]]; then gh run view --repo "$repo" --job "$job" --log-failed; fi
done <<< "$jobs"

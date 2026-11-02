#!/bin/sh
# Recaptures the visual goldens on CI's image (pictures from any other machine would differ by
# fonts and anti-aliasing): `mise run ui:update`. Runs the `ui` workflow in capture mode on the
# current branch, waits, and replaces tests/ui/goldens with its `goldens` artifact. Review the
# pictures, then commit them. Needs `gh` signed in, and the branch pushed.
set -eu
cd "$(git rev-parse --show-toplevel)"
branch=$(git branch --show-current)
before=$(gh run list --workflow ui.yml --event workflow_dispatch --branch "$branch" --limit 1 --json databaseId --jq '.[0].databaseId // 0')
gh workflow run ui.yml --ref "$branch" -f capture=true
run=$before
while [ "$run" = "$before" ]; do
  sleep 5
  run=$(gh run list --workflow ui.yml --event workflow_dispatch --branch "$branch" --limit 1 --json databaseId --jq '.[0].databaseId // 0')
done
echo "ui run $run: $(gh run view "$run" --json url --jq .url)"
# The structural checks may fail on the way (that is for you to read); the pictures are written either way.
gh run watch "$run" >/dev/null || true
rm -rf tests/ui/goldens
gh run download "$run" --name goldens --dir tests/ui/goldens
echo "goldens: $(find tests/ui/goldens -name '*.png' | wc -l) pictures in tests/ui/goldens"

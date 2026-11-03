#!/bin/sh
# Approve the key screens as they look now (`mise run ui:approve`): runs the ui-gates workflow in capture mode on the
# current branch, waits, and replaces tests/ui/gates/goldens/linux with its pictures. Review the changed pictures
# (git diff --stat, then open them), then commit them with the change that made them. Pictures from any other
# system differ by fonts and anti-aliasing, so only CI's are committed. Needs `gh` signed in and the branch pushed.
set -eu
cd "$(git rev-parse --show-toplevel)"
branch=$(git branch --show-current)
before=$(gh run list --workflow ui-gates.yml --event workflow_dispatch --branch "$branch" --limit 1 --json databaseId --jq '.[0].databaseId // 0')
gh workflow run ui-gates.yml --ref "$branch" -f capture=true
run=$before
while [ "$run" = "$before" ]; do
  sleep 5
  run=$(gh run list --workflow ui-gates.yml --event workflow_dispatch --branch "$branch" --limit 1 --json databaseId --jq '.[0].databaseId // 0')
done
echo "ui-gates run $run: $(gh run view "$run" --json url --jq .url)"
# The other gates may fail on the way (that is for you to read); the pictures are written either way.
gh run watch "$run" >/dev/null || true
rm -rf tests/ui/gates/goldens/linux
gh run download "$run" --name gates-goldens --dir tests/ui/gates/goldens/linux
echo "goldens: $(find tests/ui/gates/goldens/linux -name '*.png' | wc -l) pictures; review with: git status tests/ui/gates/goldens"

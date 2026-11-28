#!/bin/sh
# Approve the key screens as they look now (`mise run ui:approve`): runs the check workflow with `capture` on the
# current branch (its gates jobs alone, on the machines that compare the pictures), waits, and replaces
# tests/ui/gates/goldens/linux with its pictures: the screens.spec.mjs `pictured` ones only, so no golden outlives its
# comparison. Review the changed pictures (git diff --stat, then open them), then
# commit them with the change that made them. Pictures from any other system differ by fonts and anti-aliasing, so only
# CI's are committed. Needs `gh` signed in and the branch pushed.
set -eu
cd "$(git rev-parse --show-toplevel)"
branch=$(git branch --show-current)
before=$(gh run list --workflow check.yml --event workflow_dispatch --branch "$branch" --limit 1 --json databaseId --jq '.[0].databaseId // 0')
gh workflow run check.yml --ref "$branch" -f capture=true
run=$before
while [ "$run" = "$before" ]; do
  sleep 5
  run=$(gh run list --workflow check.yml --event workflow_dispatch --branch "$branch" --limit 1 --json databaseId --jq '.[0].databaseId // 0')
done
echo "check run $run (capture): $(gh run view "$run" --json url --jq .url)"
# The other gates may fail on the way (that is for you to read); the pictures are written either way.
gh run watch "$run" >/dev/null || true
# The run splits the screens over three machines, each uploading the pictures it took (gates-goldens-1..3; a machine
# with no screen tests uploads none): together, all of them.
# Inside the repository (gitignored): gh on Windows can't write to Git Bash's /tmp.
parts=tests/ui/test-results/goldens-parts
rm -rf "$parts"
gh run download "$run" --pattern 'gates-goldens-*' --dir "$parts"
rm -rf tests/ui/gates/goldens/linux
mkdir -p tests/ui/gates/goldens/linux
for part in "$parts"/*/; do cp -R "$part". tests/ui/gates/goldens/linux/; done
rm -rf "$parts"
echo "goldens: $(find tests/ui/gates/goldens/linux -name '*.png' | wc -l) pictures; review with: git status tests/ui/gates/goldens"

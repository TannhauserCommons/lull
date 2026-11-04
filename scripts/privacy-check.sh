#!/bin/sh
# Nothing in the repository or its history points to the owner (AGENTS.md, Boundaries): every commit's time is UTC, and
# no file or commit message names what PRIVATE_PATTERNS lists (the secret of that name: extended regexes, one a line,
# matched without case). The list lives only in the secret: kept here, it would publish what it guards. Prints where,
# never what. Run by .github/workflows/privacy.yml; `PRIVATE_PATTERNS=... sh scripts/privacy-check.sh` locally.
set -eu
fail=0

# a commit's time zone says where its author's clock was
bad=$(git log --format='%h %ai %ci' | awk '$4 != "+0000" || $7 != "+0000" { print $1 }')
if [ -n "$bad" ]; then
  echo "Commits not in UTC (make them again with TZ=UTC):"; echo "$bad"; fail=1
fi

if [ -z "${PRIVATE_PATTERNS:-}" ]; then
  echo "PRIVATE_PATTERNS isn't set: files and messages not checked"; exit "$fail"
fi
# each line is one -e pattern (no temporary file: a native git on Windows can't open a POSIX temp path)
set --
while IFS= read -r p; do
  case "$p" in *[![:space:]]*) set -- "$@" -e "$p" ;; esac
done <<PATTERNS
$PRIVATE_PATTERNS
PATTERNS
[ "$#" -gt 0 ] || { echo "PRIVATE_PATTERNS holds no patterns"; exit 1; }

# grep's status 1 is "nothing found"; anything above it is a failure to look, which must not pass as clean
files=$(git grep -I -l -i -E "$@" HEAD -- .) || [ "$?" = 1 ] || { echo "git grep failed"; exit 2; }
if [ -n "$files" ]; then
  echo "Files naming something private:"; echo "$files" | sed 's/^HEAD://'; fail=1
fi
log=$(git log -z --format='%h %B' | tr '\n' ' ' | tr '\0' '\n')
msgs=$(printf '%s\n' "$log" | grep -i -E "$@") || [ "$?" = 1 ] || { echo "grep failed"; exit 2; }
if [ -n "$msgs" ]; then
  echo "Commit messages naming something private:"; printf '%s\n' "$msgs" | cut -d' ' -f1; fail=1
fi
[ "$fail" = 0 ] && echo "Nothing private in $(git ls-files | wc -l) files and $(git rev-list --count HEAD) commits"
exit "$fail"

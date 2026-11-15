#!/bin/sh
# Prints an experiment's RESULT block (its verdict and evidence lines) and fails unless the verdict is PASS.
#   sh scripts/spike/result.sh e1|e2|e3
set -eu
cd "$(dirname "$0")/../.."
. scripts/spike/lib.sh

exp=${1:?usage: result.sh e1|e2|e3}
dir=$SPIKE/$exp
name=$(printf '%s' "$exp" | tr '[:lower:]' '[:upper:]')
result=$(cat "$dir/verdict.txt" 2>/dev/null || echo "FAIL (no verdict: a step stopped before it)")
{
  echo "RESULT $name: $result"
  sed 's/^/  /' "$dir/evidence.txt" 2>/dev/null || true
  echo "END RESULT $name"
} | tee "$dir/result.txt"
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  { echo '```'; cat "$dir/result.txt"; echo '```'; } >> "$GITHUB_STEP_SUMMARY"
fi
[ "$result" = PASS ]

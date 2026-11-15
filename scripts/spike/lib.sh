# Shared by the spike scripts: sourced after `cd` to the repository root, never run. POSIX sh.
# Each experiment keeps its files under $SPIKE/<exp>/: evidence.txt (the lines of its RESULT block), raw.log (the commands'
# own output), verdict.txt (PASS or FAIL). result.sh prints the block and fails the job unless the verdict is PASS.
SPIKE=${SPIKE:-${RUNNER_TEMP:-/tmp}/spike}
export LC_ALL=C

# Call after setting EXP (e1, e2 or e3).
spike_init() {
  OUT=$SPIKE/$EXP
  RAW=$OUT/raw.log
  mkdir -p "$OUT"
  : >> "$RAW"
}

# One evidence line (or several): shown in the log and kept for the RESULT block.
note() { printf '%s\n' "$*" | tee -a "$OUT/evidence.txt"; }

verdict() { printf '%s\n' "$1" > "$OUT/verdict.txt"; }

# Runs a command, keeps its output in raw.log and shows it; returns the command's status.
cap() {
  printf '\n$ %s\n' "$*" | tee -a "$RAW"
  "$@" > "$RAW.part" 2>&1 && status=0 || status=$?
  tee -a "$RAW" < "$RAW.part"
  rm -f "$RAW.part"
  return "$status"
}

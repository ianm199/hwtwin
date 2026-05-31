#!/bin/bash
# Neural Engine stimulus probe: runs the conv CoreML model on the ANE and
# samples all keys, to find which respond to ANE load. Appends to the dataset.
set -u

HERE="$(cd "$(dirname "$0")" && pwd)"
BIN="$HERE/target/release/smc_reader"
PY="$HERE/ane-venv/bin/python"
OUT="$HERE/smc_mapping"
SAMPLES="$OUT/samples.jsonl"
LOG="$OUT/run_ane.log"
: > "$LOG"

cleanup() { pkill -P $$ 2>/dev/null; pkill -f ane_stress 2>/dev/null; }
trap cleanup EXIT INT TERM
log() { echo "[$(date '+%H:%M:%S')] $*" | tee -a "$LOG"; }

DUR=35; COOL=45; PRE=10; BASE=20; IV=3; REPS=2

sample() {
  local stim="$1" phase="$2" rep="$3" dur="$4"
  local end=$(( $(date +%s) + dur ))
  while [ "$(date +%s)" -lt "$end" ]; do
    printf '{"t":%s,"stim":"%s","phase":"%s","rep":%s,"sensors":%s}\n' \
      "$(date +%s)" "$stim" "$phase" "$rep" "$("$BIN" json)" >> "$SAMPLES"
    sleep "$IV"
  done
}

stress_ane() { timeout "$1" "$PY" "$HERE/ane_stress.py" "$1" >/dev/null 2>&1 & }

log "global baseline"
sample baseline baseline 500 "$BASE"

for rep in $(seq 1 "$REPS"); do
  log "ane rep $rep: idle pre"
  sample ane pre "$rep" "$PRE"
  log "ane rep $rep: LOAD"
  stress_ane "$DUR"
  sample ane load "$rep" "$DUR"
  pkill -f ane_stress 2>/dev/null
  log "ane rep $rep: cooldown"
  sample ane cooldown "$rep" "$COOL"
done

log "running analysis"
python3 "$HERE/analyze.py" "$OUT" >> "$LOG" 2>&1
python3 "$HERE/peripherals.py" "$OUT" >> "$LOG" 2>&1
log "DONE"

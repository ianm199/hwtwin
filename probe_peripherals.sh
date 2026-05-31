#!/bin/bash
# Scriptable peripheral stimulus sweep: Wi-Fi radio (sustained download) and
# audio (speaker amp). Appends to the existing dataset so we can find which keys
# — especially the unmapped o*/D* families — respond to non-compute subsystems.
# Reversible: saves and restores output volume. Bounded and self-cleaning.
set -u

HERE="$(cd "$(dirname "$0")" && pwd)"
BIN="$HERE/target/release/smc_reader"
OUT="$HERE/smc_mapping"
SAMPLES="$OUT/samples.jsonl"
LOG="$OUT/run_periph.log"
: > "$LOG"

ORIG_VOL=""
cleanup() {
  pkill -P $$ 2>/dev/null
  pkill -f afplay 2>/dev/null
  [ -n "$ORIG_VOL" ] && osascript -e "set volume output volume $ORIG_VOL" 2>/dev/null
}
trap cleanup EXIT INT TERM
log() { echo "[$(date '+%H:%M:%S')] $*" | tee -a "$LOG"; }

DUR=35; COOL=45; PRE=12; BASE=20; IV=3; REPS=2
SOUND="/System/Library/Sounds/Submarine.aiff"
WIFI_URL="https://speed.cloudflare.com/__down?bytes=26214400"

ORIG_VOL=$(osascript -e 'output volume of (get volume settings)' 2>/dev/null)

sample() {
  local stim="$1" phase="$2" rep="$3" dur="$4"
  local end=$(( $(date +%s) + dur ))
  while [ "$(date +%s)" -lt "$end" ]; do
    local ts sensors
    ts=$(date +%s)
    sensors=$("$BIN" json)
    printf '{"t":%s,"stim":"%s","phase":"%s","rep":%s,"sensors":%s}\n' \
      "$ts" "$stim" "$phase" "$rep" "$sensors" >> "$SAMPLES"
    sleep "$IV"
  done
}

have_net() { curl -s --max-time 4 -o /dev/null "https://speed.cloudflare.com/__down?bytes=1000"; }

stress_wifi() {
  local dur="$1"
  ( local end=$(( $(date +%s) + dur ))
    while [ "$(date +%s)" -lt "$end" ]; do curl -s -o /dev/null --max-time "$dur" "$WIFI_URL"; done ) &
}

stress_audio() {
  local dur="$1"
  osascript -e 'set volume output volume 50' 2>/dev/null
  ( local end=$(( $(date +%s) + dur ))
    while [ "$(date +%s)" -lt "$end" ]; do afplay "$SOUND" 2>/dev/null; done ) &
}

run_stim() {
  local name="$1" fn="$2"
  for rep in $(seq 1 "$REPS"); do
    log "$name rep $rep: idle pre"
    sample "$name" pre "$rep" "$PRE"
    log "$name rep $rep: LOAD"
    "$fn" "$DUR"
    sample "$name" load "$rep" "$DUR"
    pkill -P $$ 2>/dev/null; pkill -f afplay 2>/dev/null
    log "$name rep $rep: cooldown"
    sample "$name" cooldown "$rep" "$COOL"
  done
}

log "global baseline"
sample baseline baseline 300 "$BASE"

if have_net; then
  run_stim wifi stress_wifi
else
  log "SKIP wifi: no connectivity"
fi
run_stim audio stress_audio

[ -n "$ORIG_VOL" ] && osascript -e "set volume output volume $ORIG_VOL" 2>/dev/null

log "running analysis"
python3 "$HERE/analyze.py" "$OUT" >> "$LOG" 2>&1
python3 "$HERE/peripherals.py" "$OUT" >> "$LOG" 2>&1
log "DONE"

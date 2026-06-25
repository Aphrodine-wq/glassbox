#!/usr/bin/env bash
#
# run.sh — the wrap-a-foreign-agent demo.
#
# Governs an agent we did NOT write (demo/agent.py) as it's prompt-injected into
# trying to exfiltrate ~/.ssh/id_rsa to evil.example. Runs in shadow (observe)
# then enforce (block), self-verifies behaviour vs the declared plan each time,
# and prints the audit trail. Tools are sandboxed — nothing real ever leaks.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
export GLASSBOX_BIN="$HERE/../target/release/glassbox"
export GLASSBOX_ENVELOPE_FILE="$HERE/governance.json"

# stage the benign notes (with the planted injection) inside the allowed scope
mkdir -p "$HOME/demo"
cp "$HERE/notes.txt" "$HOME/demo/notes.txt"

for MODE in shadow enforce; do
  echo
  echo "════════════════════ MODE: $MODE ════════════════════"
  GLASSBOX_MODE="$MODE" DEMO_TRACE="$HERE/trace.$MODE.jsonl" python3 "$HERE/agent.py"
done

echo
echo "════════ AUDIT TRAIL (~/.glassbox/decisions.jsonl · last 6) ════════"
tail -6 "$HOME/.glassbox/decisions.jsonl" 2>/dev/null | node -e '
require("readline").createInterface({input:process.stdin}).on("line",l=>{
  try{const e=JSON.parse(l);const m=e.blocked?"⛔":"✓";
  console.log("   "+m+" ["+(e.mode||"").padEnd(7)+"] "+String(e.action||"").slice(0,40).padEnd(40)+"  "+String(e.reason||"").slice(0,38));}catch(_){}})'
echo

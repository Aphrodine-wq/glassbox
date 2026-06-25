#!/usr/bin/env python3
"""govern.py — the ENTIRE integration surface for governing a foreign agent.

Any agent that calls govern() before executing a tool is governed by Glass Box.
No other change to the agent is required. This is the wedge: one function.

Returns (allowed, blocked, reason, provenance_id):
  - blocked        = the gate's verdict (true if any rail refused), independent of mode
  - allowed        = whether the agent may proceed: always true in shadow (observe-only),
                     `not blocked` in enforce. So the SAME verdict, two postures.
"""
import json
import os
import subprocess

GLASSBOX = os.environ.get(
    "GLASSBOX_BIN",
    os.path.expanduser("~/Projects/walt/glassbox/target/release/glassbox"),
)


def govern(action: str, target: str, agent: str = "demo-agent"):
    req = json.dumps({"action": action, "target": target, "agent": agent})
    try:
        out = subprocess.run(
            [GLASSBOX, "gate-json"], input=req, capture_output=True, text=True, timeout=10
        )
        r = json.loads(out.stdout)
    except Exception:
        return True, False, "(gate unavailable — fail-open)", ""  # never break the agent

    blocked = bool(r.get("blocked", False))
    mode = os.environ.get("GLASSBOX_MODE", "shadow").strip().lower()
    allowed = True if mode != "enforce" else (not blocked)
    return allowed, blocked, r.get("reason", ""), r.get("provenance_id", "")

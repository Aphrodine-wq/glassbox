#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = ["mcp>=1.2.0"]
# ///
"""Glass Box MCP server — govern any MCP agent's proposed action through the rails.

One tool, `glassbox_gate`, shells out to the `glassbox gate-json` binary (the
generic, agent-agnostic adapter) and returns the trust-card + verdicts. The Rust
binary stays the single source of truth for the gating logic; this shim only
translates MCP calls into the generic protocol, so MCP, the Claude Code hook, and
the CLI all hit the identical code path. Zero Rust dependencies added — this drops
into ~/.claude/.mcp.json exactly like the tessera/vault servers (uv run --script).

Shadow by default: nothing is blocked. The value is SEEING the governed decision
(the card + which value/policy would fire) at the moment of action.
"""
import json
import os
import subprocess

from mcp.server.fastmcp import FastMCP

mcp = FastMCP("glassbox")

# The thin wrapper at ~/bin/glassbox execs the release binary. Override with
# GLASSBOX_BIN if you run the debug build or a relocated install.
BIN = os.environ.get("GLASSBOX_BIN", os.path.expanduser("~/bin/glassbox"))


@mcp.tool()
def glassbox_gate(action: str, target: str = "unknown", agent: str = "mcp") -> str:
    """Govern a proposed action through the Glass Box rails (values + safety) and
    return the trust-card plus structured verdicts and provenance.

    `action` is the rendered proposed action (e.g. "git push origin main --force"
    or "reprice loyal client"); `target` is what it acts on (a path, a repo,
    a person). In shadow mode nothing is blocked — the result reports what the gate
    WOULD do and exactly why (which value/policy fires, where it escalates), so an
    agent or a human can see the governed decision at the moment of action. Returns
    the gate-json response: decision, blocked, verdicts, card, provenance_id."""
    payload = json.dumps({"action": action, "target": target, "agent": agent})
    try:
        out = subprocess.run(
            [BIN, "gate-json"],
            input=payload,
            capture_output=True,
            text=True,
            timeout=15,
        )
    except FileNotFoundError:
        return json.dumps({"error": f"glassbox binary not found at {BIN}"})
    except subprocess.TimeoutExpired:
        return json.dumps({"error": "glassbox gate timed out"})
    if out.returncode != 0 and not out.stdout.strip():
        return json.dumps({"error": out.stderr.strip() or "glassbox gate failed"})
    return out.stdout.strip()


if __name__ == "__main__":
    mcp.run()

# The Glass Box — Usage

A governance gate for AI agents. It reads a proposed action, runs it through two
rails (**safety** = irreversible, **values** = wrong), and renders a plain-language
trust-card. In the default **shadow** mode it shows you what it *would* do and
never blocks; in **enforce** mode it can deny.

No Tessera and no Python are required — the values rail runs natively out of the
box. (If a Tessera install is present, the gate upgrades to it automatically.)

---

## Install

```bash
git clone https://github.com/Aphrodine-wq/glassbox
cd glassbox
cargo build --release
# binary: ./target/release/glassbox
```

Put it on your PATH so the examples below work verbatim:

```bash
install -m755 target/release/glassbox ~/bin/glassbox   # or: cp to anywhere on $PATH
glassbox demo
```

Requires a Rust toolchain (`rustup`, stable). Zero runtime dependencies.

---

## Quickstart

```bash
# Govern one action (enforce mode; exits 1 if blocked)
glassbox gate "git push origin main --force"

# See six example actions governed and rendered
glassbox demo

# Prove the numbers yourself
glassbox eval
glassbox eval --values     # also exercises the values rail
```

`glassbox gate` prints the trust-card and sets its exit code from the decision —
handy in scripts and pre-commit hooks.

---

## CLI reference

| Command | What it does |
| --- | --- |
| `glassbox gate "<action>" [target]` | Govern one action in **enforce** mode. Prints the card; exit `1` if blocked, `0` if allowed. |
| `glassbox gate-json` | The generic API. Reads a JSON request on stdin, prints the full `GateResponse` JSON. Mode defaults to shadow. |
| `glassbox hook` | Claude Code PreToolUse adapter. Reads the tool-call JSON on stdin, prints hook-protocol JSON. |
| `glassbox demo` | Runs six representative actions through the gate and renders each card. |
| `glassbox watch` | Live stream of cards as decisions land in `~/.glassbox/decisions.jsonl`. Run in a second pane. |
| `glassbox status` | Recent decisions, the would-block count, and (if Tessera is active) the provenance link. |
| `glassbox eval [--values]` | Reproducible benchmark: coverage + latency. `--values` adds the values rail. |

---

## The `gate-json` protocol

This is the agent-agnostic core. Anything that can describe a proposed action can
be governed.

**Request** (stdin):

```json
{
  "action": "git push --force",   // required: the rendered action
  "target": "git",                // optional: what it acts on (path/repo/person); default "shell"
  "agent":  "my-agent",           // optional: who is acting; default "unknown"
  "mode":   "shadow"              // optional: "shadow" (default) or "enforce"
}
```

**Response** (stdout, one line):

```json
{
  "t": 1781634897331,
  "action": "git push --force",
  "target": "git",
  "agent": "my-agent",
  "mode": "shadow",
  "blocked": true,                 // what the RAILS decided (rail refused)
  "decision": "would-refuse",      // what the MODE allowed: would-refuse | would-allow | refused | allowed
  "reason": "safety rail: contains forbidden substring '--force'",
  "verdicts": [
    {"rail":"values","refused":false,"reason":"clean","policy":""},
    {"rail":"safety","refused":true,"reason":"contains forbidden substring '--force'","policy":"Irreversible"}
  ],
  "provenance": {
    "source":"glassbox/safety","value":"reversibility","intent":"guard_action",
    "policy":"Irreversible","escalation":"a human clears irreversible calls"
  },
  "provenance_id": "gbx_1781634897331_0000",
  "card": "  ┌─ GLASS BOX ─ moment of action ─ [SHADOW] ─ …"
}
```

The key separation: **`blocked`** is what the rails decided; **`decision`** is what
the *mode* allowed. In shadow, `blocked` can be `true` while the action still
proceeds — that's what makes shadow meaningful.

Example:

```bash
echo '{"action":"DROP TABLE users","target":"db","agent":"etl"}' | glassbox gate-json
```

---

## Integrations

### Claude Code (PreToolUse hook)

Glassbox governs Claude Code governing itself. Add a PreToolUse hook to
`~/.claude/settings.json`:

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash|Write|Edit",
        "hooks": [
          { "type": "command", "command": "\"$HOME/bin/glassbox\" hook", "timeout": 20 }
        ]
      }
    ]
  }
}
```

In **shadow** (the default) the hook emits a one-line `systemMessage` for each
governed action and blocks nothing:

```json
{"systemMessage":"Glass Box · SHADOW · WOULD-REFUSE (Irreversible) · git push --force · values✓ safety⛔ · see: glassbox watch · gbx_…"}
```

To **enforce**, set `GLASSBOX_MODE=enforce` in the hook's environment. A blocked
action then returns the deny protocol:

```json
{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"Glass Box governance — safety rail: contains forbidden substring 'rm -rf'"}}
```

Run `glassbox watch` in a second pane to see the full cards live. Enforce is
opt-in, never the default.

### MCP (any MCP-native agent)

`mcp/glassbox_mcp.py` exposes one tool, `glassbox_gate(action, target, agent)`,
which shells out to `glassbox gate-json` and returns the response. It's a `uv`-run
script with no Rust dependency. Register it in your MCP config and call it before
acting.

---

## Customizing the rails

### Safety (the irreversible floor)

The forbidden patterns live in **`agents/safety.t.md`** as readable lines:

```
forbid contains "rm -rf"
forbid contains "drop table"
forbid contains "dd if="
```

Add or remove lines to change the floor — no recompile. The patterns are parsed
once per process. Point the gate at a different file with `GLASSBOX_SAFETY_FILE`.
If no file is found, a built-in fallback keeps the 13 default patterns active, so
a fresh clone always has a floor.

### Values (the wrongness rail)

By default the values rail uses a native, in-process oracle that refuses
extraction (gouge, exploit, squeeze, overcharge), deception (defraud, deceive),
repricing a loyal relationship, and unfair markup on a known counterparty — and
passes fair business (a refund, a standard deposit, repricing a new tier). A
keyword pre-screen means non-money actions skip it entirely.

**Optional Tessera upgrade.** If you have the WALT Tessera runtime and a
`conscience.t.md`, the gate uses your richer Conscience agent automatically when
both are found on disk. Override the locations with `GLASSBOX_TESSERA_BIN` and
`GLASSBOX_CONSCIENCE`. The values rail **fails open**: if the oracle errors or
times out, the action is allowed (safety, which never fails, stays the floor).

---

## Environment variables

| Variable | Default | Effect |
| --- | --- | --- |
| `GLASSBOX_MODE` | `shadow` | `shadow` (never blocks) or `enforce` (blocks on refusal). |
| `GLASSBOX_SAFETY_FILE` | `~/Projects/walt/glassbox/agents/safety.t.md` | Path to the safety patterns file. Falls back to built-in patterns if absent. |
| `GLASSBOX_TESSERA_BIN` | WALT default path | Tessera binary. If it (and the conscience file) exist, the values rail upgrades to Tessera. |
| `GLASSBOX_CONSCIENCE` | WALT default path | The `conscience.t.md` agent for the Tessera values oracle. |

---

## Data & exit codes

- **Decisions log:** every decision appends to `~/.glassbox/decisions.jsonl` (one
  JSON record per line) — the source `glassbox watch` and `glassbox status` read.
- **Exit codes:** `glassbox gate` returns `0` (allowed) or `1` (blocked);
  `gate-json` returns `1` on a malformed request; `eval` returns `1` only on a
  floor regression against its own corpus.

---

## Verify it works

```bash
cargo test           # full suite
glassbox eval        # 12/12 destructive caught, 0/14 false positives, misses named
glassbox demo        # watch the rails decide on real actions
```

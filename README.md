# The Glass Box

> A governance trust-layer for agents. Rust. The values-check *is* the card a
> person reads at the moment the agent acts.

Every agent framework is a black box with a debugger bolted on. **The Glass Box
makes an agent's actions legible and governed at the moment they happen**: each
proposed action runs through two readable rails and renders as a plain-language
**trust-card** — and is *refused* if a rail says no. Not a JSON trace for an
engineer to debug after the fact. A card a human reads at the instant of action.

```
  ▸ Force-push to main (IRREVERSIBLE)
  ┌─ GLASS BOX ─ moment of action ─────────────
    perceive  git push origin main --force
    values  ✓ clean
    safety  ⛔ refused  contains forbidden substring '--force'  [Irreversible]
    decision  ⛔ BLOCKED  safety rail: contains forbidden substring '--force'
  └──────────────────────────────────────────
```

## Two rails — both readable markdown

Governance is not reimplemented in Rust — it's delegated to **Tessera `.t.md`**
files you can open, read, and edit. Blocked if *either* rail refuses:

| Rail | Refuses what is… | Source | Path |
| --- | --- | --- | --- |
| **safety** | *irreversible* — rm -rf, force push, hard reset, table drop | `agents/safety.t.md` (patterns read at runtime) | in-process, always on |
| **values** | *wrong* — extractive, unfair, repricing a loyal client | `~/Projects/walt/mind/conscience.t.md` (Conscience) | Tessera subprocess |

The two rails answer different questions. A force-push violates no *value* — it's
not unfair — it's just unrecoverable. A values-only gate waves it through. You
need both.

## Built for the live agent

This is the Rust rewrite of the Python prototype (`~/Projects/walt/candor`),
built to run on **every tool call** without slowing the agent:

- **Safety runs in-process** — pure string match, **~9ms**, no subprocess, no
  failure mode. The hard floor that's always on.
- **Values is pre-screened** — the Tessera subprocess only fires when the action
  contains a values keyword (price/charge/casey/…); the 99% of commands with
  nothing to do with money pay nothing.
- **Values fails *open*** — a rare Tessera hiccup logs + emits a Nerve event but
  does not block. A missed refusal during an outage is recoverable; a bricked
  agent is not. Safety, which never fails, stays the floor.

## Use

```bash
cargo build --release
glassbox demo                       # the proof: 4 of 6 actions blocked, each readably
glassbox gate "git push --force"    # govern one action
glassbox status                     # recent decisions
```

## Wire it into Claude Code (the first integration)

The Glass Box's first job is **Claude Code governing itself.** Add a PreToolUse
hook so every Bash/Edit/Write passes through the gate (it *blocks* the dangerous
ones with a readable reason shown to you and the model):

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

Scoped to the tools that can do damage (not Read/Grep). Every decision streams a
`glassbox:*` Nerve event and lands in `~/.glassbox/decisions.jsonl`.

## Roadmap

- **v1 (here):** Rust gate + trust-card + Claude Code PreToolUse hook.
- **v2 — generalize:** fork **Goose** (Block, Apache-2.0, Rust, MCP-native) and
  slot the rails into its `PermissionInspector` seam to govern *any* on-machine
  agent.
- **v3 — the face:** live card stream as a TUI, then a shareable web UI.

## License

MIT. Built on Tessera (the WALT runtime). v2 builds on Goose (Apache-2.0). We do
not build on Screenpipe (commercial source-available) or Khoj (AGPL).

> Name note: "The Glass Box" also names an unrelated Solana project; it's a
> generic interpretability term too. Chosen for community resonance with eyes open.

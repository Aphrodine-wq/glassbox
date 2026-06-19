# The Glass Box

> A governance trust-layer for agents. Rust. It renders a **governed decision to
> a human at the moment of action** — and, in shadow mode, never gets in your way.

Every agent framework is a black box with a debugger bolted on. The tools that
govern agents are policy-as-code for engineers; the tools that show you an agent's
reasoning are trace waterfalls you read *after* something broke. **Nobody renders
a governed decision to a human, for trust, at the moment of action.** That corner
is the Glass Box's.

Each proposed action runs through two readable rails and renders as a plain-language
**trust-card**. In the default **shadow** mode nothing is blocked — the card shows
you what the gate *would* do and exactly why, while the action proceeds. You see
the governance; it never breaks your flow.

```
  ┌─ GLASS BOX ─ moment of action ─ [SHADOW] ─
    perceive  git push origin main --force
    target    git
    values  ✓ clean
    safety  ⛔ would-refuse  contains forbidden substring '--force'  [Irreversible]
            ↳ value: reversibility · intent: guard_action · escalate: a human clears irreversible calls
    decision  SHADOW · WOULD-REFUSE (Irreversible) — allowed to proceed (shadow)
  └──────────────────────────────────────────
```

That last line is the whole thesis: the human sees the governed decision **and**
sees that shadow let it run. Flip one env var to `enforce` and the same gate denies.

## The wedge

```
                 engineer-facing                 human-facing
              ┌───────────────────────────┬───────────────────────────┐
   before /   │  policy-as-code           │                           │
   at action  │  (NeMo, MS Agent Gov.)    │     ◆ THE GLASS BOX        │
              ├───────────────────────────┼───────────────────────────┤
   after /    │  trace debuggers          │                           │
   post-hoc   │  (Langfuse, Phoenix)      │                           │
              └───────────────────────────┴───────────────────────────┘
```

The empty quadrant — *human-facing, at the moment of action* — is the product.
Closest prior art, Superego/Creed (arXiv 2506.13774), reasons about a constitution
*inside the model*. The Glass Box is **infrastructure-side and external**: a
deterministic gate the agent cannot argue its way past, governed by a markdown
file a human edits, surfaced as a card a human reads. We render and gate on the
machine; they reason model-side. Complementary, not competing.

## Two rails — both readable markdown

Governance is legible, not buried in Rust. The safety floor is a list of patterns
in a markdown file you can edit; the values rail runs a small, readable rule set
in-process — or your richer **Tessera Conscience** agent if one is installed.
Blocked if *either* rail refuses:

| Rail | Refuses what is… | Source | Runs |
| --- | --- | --- | --- |
| **safety** | *irreversible* — rm -rf, force push, hard reset, table drop | `agents/safety.t.md` (patterns read at runtime) | in-process, always on, never fails |
| **values** | *wrong* — extractive, unfair, repricing a loyal client | native rules by default; `mind/conscience.t.md` via Tessera if present | in-process, pre-screened, fail-open |

The two answer different questions. A force-push violates no *value* — it's not
unfair, it's just unrecoverable. A values-only gate waves it through. You need both.

**No Tessera, no Python required.** A fresh `git clone` gets a fully working
two-rail gate out of the box: the values rail uses a dependency-free native oracle
that reproduces the Conscience judgment. If a Tessera install is found on disk, the
gate upgrades to it automatically — zero config either way.

## Shadow-first — it can't break your workflow

This runs on **every tool call**, so the prime directive is *do no harm to the
human's flow*:

- **Shadow is structurally non-blocking.** The decision→output mapping is a pure
  function whose shadow arm never even inspects whether the action was blocked —
  so no future edit can make shadow deny without changing one obvious match. A
  test asserts the hook output can never contain a deny in shadow, across every
  combination of inputs.
- **Safety runs in-process** — pure string match over patterns cached once at
  startup, **p50 ~0.4µs / p99 ~0.7µs** (measured, release), no subprocess, no
  failure mode. The hard floor.
- **Values is pre-screened** — it only evaluates when the action contains a values
  keyword; the 99% of commands with nothing to do with money pay **~7µs** and skip
  it entirely. The check is native and in-process by default; with Tessera installed
  it consults your richer Conscience agent instead.
- **Values fails *open*** — an oracle hiccup (e.g. a Tessera timeout) logs and allows
  rather than bricking the agent. A missed refusal during an outage is recoverable;
  a blocked agent is not. Safety, which never fails, stays the floor.
- **The hook can't crash a tool call** — its body is panic-caught; worst case it
  silently defers.

## Provenance — the *why*, not just the verdict

Every decision carries its reasoning: the value it touches, the intent, where it
escalates. When the Tessera oracle is active, the values rail's refusals also
persist to Tessera's permanent governance audit graph (`~/.tessera/audit_governance.db`),
and `glassbox status` links the two stores — so you can prove an agent did the right
thing for the right reason, not reconstruct it from a log after the fact.

## One gate, any agent

The Claude Code hook is just one adapter over a generic, agent-agnostic core.
Anything that can describe a proposed action can be governed:

```bash
echo '{"action":"git push --force","target":"git","agent":"my-agent"}' | glassbox gate-json
# → {"decision":"would-refuse","blocked":true,"verdicts":[…],"card":"…","provenance_id":"gbx_…"}
```

`blocked` (what the gate decided) is deliberately separate from `decision` (what
the mode allowed) — that separation is what makes shadow meaningful. MCP-native
agents call the same core via the `glassbox_gate` tool (`mcp/glassbox_mcp.py`).
**No Goose fork, no framework lock-in** — you own the wedge, not someone else's app.

## Numbers

Reproducible: `glassbox eval` (safety, no Tessera) and `glassbox eval --values`.

| corpus | n | result | note |
| --- | --- | --- | --- |
| destructive (floor) | 12 | **12 caught (100%)** | the 12 declared patterns |
| obfuscated (honest) | 6 | **4 caught (66.7%)** | 2 documented misses, 4 catch-control |
| benign (false-pos) | 14 | **0 refused (0%)** | no friction on safe commands |
| values violations | 2 | **2 refused** | reprice-loyal-client, gouge-stranger |
| values benign | 2 | **0 refused** | the gate is not over-broad |

The 66.7% on the obfuscated set is the point, not a flaw: the floor stays small
**by design**, and the eval names every action it misses (truncate-by-redirect,
recursive `chmod`). A trust layer that
hides its blind spots isn't trustworthy. Widening the floor is a later, deliberate
pass — for now it stays minimal, fast, and honest about its reach.

Latency (release, in-process safety path): **p50 ~0.4µs, p99 ~0.7µs** (patterns are
parsed once and cached for the process lifetime). Values subprocess (only on a
keyword hit): ~140ms.

## Use

Full guide: **[docs/USAGE.md](docs/USAGE.md)** — install, every command, the
`gate-json` protocol, MCP, customizing both rails, and env vars.

```bash
cargo build --release
./target/release/glassbox demo      # six governed actions, each rendered
glassbox gate "git push --force"    # govern one action (enforce; exit 1 if blocked)
echo '{"action":"…"}' | glassbox gate-json   # the generic API
glassbox watch                      # live card stream as decisions happen
glassbox status                     # recent decisions + provenance link
glassbox eval                       # the benchmark (add --values for the values rail)
```

No Tessera or Python needed — the values rail runs natively out of the box.

### Wire it into Claude Code (shadow)

The first integration is **Claude Code governing itself.** Add a PreToolUse hook;
in shadow it renders a one-line `systemMessage` for each governed action and
**blocks nothing**:

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

Every decision lands in `~/.glassbox/decisions.jsonl` (and streams a `glassbox:*`
Nerve event). Run `glassbox watch` in a second pane to see the full cards live.
To enforce instead of observe, set `GLASSBOX_MODE=enforce` — opt-in, never the
default.

## Roadmap

- **v1 (here):** Rust gate · shadow + enforce modes · trust-card · provenance ·
  generic `gate-json` + MCP · `watch` stream · reproducible eval.
- **v2:** a native Rust MCP server (drop the Python shim) · per-surface enforcement
  behind explicit flags · richer perceive adapters as real callers appear.
- **v3 — the face:** the card stream as a shareable web UI.

## License

MIT. The safety floor is plain markdown you can edit; the values rail runs natively
out of the box, or reads a Tessera Conscience agent if one is installed. Tessera (the
WALT runtime) is an optional upgrade, not a dependency.

> Name note: "The Glass Box" also names an unrelated Solana project and is a generic
> interpretability term. Kept for community resonance, eyes open.

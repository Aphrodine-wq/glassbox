"""Glassbox client — report governed decisions and costs from any agent."""

from __future__ import annotations

import time
import threading
import urllib.request
import urllib.error
import json
from dataclasses import dataclass, field


@dataclass
class Decision:
    """A single governed action to report."""
    agent: str
    action: str
    blocked: bool = False
    decision: str = "allow"
    reason: str = "all rails clean"
    target: str = ""
    mode: str = "enforce"
    provenance_id: str = ""
    verdicts: list[dict] = field(default_factory=list)
    t: int = 0

    def to_dict(self) -> dict:
        return {
            "agent": self.agent,
            "action": self.action,
            "blocked": self.blocked,
            "decision": self.decision,
            "reason": self.reason,
            "target": self.target,
            "mode": self.mode,
            "provenance_id": self.provenance_id,
            "verdicts": self.verdicts,
            "t": self.t or _now_ms(),
        }


@dataclass
class CostEvent:
    """Token/cost tracking for an agent."""
    agent: str
    tokens_in: int = 0
    tokens_out: int = 0
    cost_usd: float = 0.0
    model: str = ""
    t: int = 0

    def to_dict(self) -> dict:
        return {
            "agent": self.agent,
            "tokens_in": self.tokens_in,
            "tokens_out": self.tokens_out,
            "cost_usd": self.cost_usd,
            "model": self.model,
            "t": self.t or _now_ms(),
        }


class Glassbox:
    """
    Client for the Glassbox governance server.

    Usage:
        gb = Glassbox("http://localhost:3120", "gbx_your_api_key")

        # Report a governed decision
        gb.report(agent="my-agent", action="git push", blocked=False)

        # Report a blocked action
        gb.report(agent="my-agent", action="rm -rf /", blocked=True,
                  decision="deny", reason="safety rail: forbidden")

        # Track costs
        gb.cost(agent="my-agent", tokens_in=15000, tokens_out=3000, cost_usd=0.82)

        # Use as a context manager for automatic cost tracking
        with gb.track("my-agent", model="claude-sonnet-4"):
            # ... your agent work here ...
            pass

        # Batch report
        gb.report_batch([
            Decision(agent="a", action="cmd1"),
            Decision(agent="a", action="cmd2", blocked=True, decision="deny"),
        ])

        # Async (fire-and-forget, non-blocking)
        gb.report(agent="my-agent", action="git push", async_=True)
    """

    def __init__(self, url: str, api_key: str, timeout: float = 10.0):
        self.url = url.rstrip("/")
        self.api_key = api_key
        self.timeout = timeout

    def report(self, *, async_: bool = False, **kwargs) -> dict | None:
        """Report a single governed decision."""
        d = Decision(**kwargs)
        if async_:
            self._fire_and_forget("/api/ingest/decision", d.to_dict())
            return None
        return self._post("/api/ingest/decision", d.to_dict())

    def report_decision(self, decision: Decision, async_: bool = False) -> dict | None:
        """Report a pre-built Decision object."""
        if async_:
            self._fire_and_forget("/api/ingest/decision", decision.to_dict())
            return None
        return self._post("/api/ingest/decision", decision.to_dict())

    def report_batch(self, decisions: list[Decision]) -> dict:
        """Report multiple decisions in a single request."""
        payload = [d.to_dict() for d in decisions]
        return self._post("/api/ingest/decision", payload)

    def cost(self, *, async_: bool = False, **kwargs) -> dict | None:
        """Report a cost/token event."""
        c = CostEvent(**kwargs)
        if async_:
            self._fire_and_forget("/api/ingest/cost", c.to_dict())
            return None
        return self._post("/api/ingest/cost", c.to_dict())

    def cost_batch(self, events: list[CostEvent]) -> dict:
        """Report multiple cost events in a single request."""
        payload = [e.to_dict() for e in events]
        return self._post("/api/ingest/cost", payload)

    def track(self, agent: str, model: str = "") -> _CostTracker:
        """Context manager that auto-reports a cost event on exit."""
        return _CostTracker(self, agent, model)

    # ── Convenience builders ───────────────────────────────────────────

    def allow(self, agent: str, action: str, **kw) -> dict:
        """Shorthand: report an allowed action."""
        return self.report(agent=agent, action=action, blocked=False,
                           decision="allow", reason="all rails clean", **kw)

    def block(self, agent: str, action: str, reason: str, **kw) -> dict:
        """Shorthand: report a blocked action."""
        return self.report(agent=agent, action=action, blocked=True,
                           decision="deny", reason=reason, **kw)

    # ── Internal ───────────────────────────────────────────────────────

    def _post(self, path: str, payload) -> dict:
        data = json.dumps(payload).encode()
        req = urllib.request.Request(
            self.url + path,
            data=data,
            headers={
                "Authorization": f"Bearer {self.api_key}",
                "Content-Type": "application/json",
            },
            method="POST",
        )
        try:
            with urllib.request.urlopen(req, timeout=self.timeout) as resp:
                return json.loads(resp.read())
        except urllib.error.HTTPError as e:
            body = e.read().decode() if e.fp else ""
            raise GlassboxError(e.code, body) from None

    def _fire_and_forget(self, path: str, payload):
        t = threading.Thread(target=self._post, args=(path, payload), daemon=True)
        t.start()


class _CostTracker:
    """Context manager for automatic cost tracking."""

    def __init__(self, client: Glassbox, agent: str, model: str):
        self.client = client
        self.agent = agent
        self.model = model
        self.tokens_in = 0
        self.tokens_out = 0
        self.cost_usd = 0.0
        self._start = 0

    def __enter__(self):
        self._start = _now_ms()
        return self

    def __exit__(self, *exc):
        self.client.cost(
            agent=self.agent,
            tokens_in=self.tokens_in,
            tokens_out=self.tokens_out,
            cost_usd=self.cost_usd,
            model=self.model,
            t=self._start,
            async_=True,
        )

    def add(self, tokens_in: int = 0, tokens_out: int = 0, cost_usd: float = 0.0):
        """Accumulate token/cost usage within the tracked block."""
        self.tokens_in += tokens_in
        self.tokens_out += tokens_out
        self.cost_usd += cost_usd


class GlassboxError(Exception):
    """HTTP error from the Glassbox server."""
    def __init__(self, status: int, body: str):
        self.status = status
        self.body = body
        super().__init__(f"Glassbox API error {status}: {body}")


def _now_ms() -> int:
    return int(time.time() * 1000)

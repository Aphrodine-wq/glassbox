"""Middleware wrappers for popular AI agent frameworks.

Drop-in governance for CrewAI, LangGraph, and AutoGen that auto-reports
governed decisions to a Glassbox server.
"""

from __future__ import annotations

import functools
import json
import logging
import urllib.request
import urllib.error
from typing import Any, Callable, TypeVar

from glassbox import Glassbox

logger = logging.getLogger("glassbox.middleware")

F = TypeVar("F", bound=Callable)


class _BaseGovernance:
    """Shared logic for all framework wrappers."""

    def __init__(self, url: str, api_key: str, *, agent_name: str = "agent"):
        self.gb = Glassbox(url, api_key)
        self.url = url.rstrip("/")
        self.api_key = api_key
        self.agent_name = agent_name

    # ── Gate check ────────────────────────────────────────────────────

    def check(self, action: str, target: str = "") -> dict:
        """Call the Glassbox gate and return the verdict.

        Returns a dict like ``{"decision": "allow", "blocked": False, ...}``
        or ``{"decision": "deny", "blocked": True, "reason": "..."}`` if the
        action is blocked by a governance rail.

        On network errors the call is **fail-open** — it returns allow so
        that a Glassbox outage does not freeze the agent.
        """
        payload = json.dumps({"action": action, "target": target}).encode()
        req = urllib.request.Request(
            self.url + "/api/gate",
            data=payload,
            headers={
                "Authorization": f"Bearer {self.api_key}",
                "Content-Type": "application/json",
            },
            method="POST",
        )
        try:
            with urllib.request.urlopen(req, timeout=5) as resp:
                return json.loads(resp.read())
        except Exception:
            logger.warning("Glassbox gate unreachable — failing open")
            return {"decision": "allow", "blocked": False, "reason": "gate unreachable (fail-open)"}

    # ── Reporting helpers ─────────────────────────────────────────────

    def _report(self, action: str, blocked: bool, reason: str = "",
                target: str = "", agent: str | None = None) -> None:
        """Fire-and-forget report to Glassbox."""
        try:
            self.gb.report(
                agent=agent or self.agent_name,
                action=action,
                blocked=blocked,
                decision="deny" if blocked else "allow",
                reason=reason or ("blocked by rail" if blocked else "all rails clean"),
                target=target,
                async_=True,
            )
        except Exception:
            logger.debug("Failed to report decision to Glassbox", exc_info=True)


# =====================================================================
# CrewAI
# =====================================================================

class CrewAIGovernance(_BaseGovernance):
    """Drop-in governance for CrewAI agents.

    Usage::

        from glassbox.middleware import CrewAIGovernance
        gov = CrewAIGovernance("http://localhost:3120", "gbx_key")

        # Wrap a CrewAI tool
        @gov.govern
        def my_tool(input: str) -> str:
            ...

        # Or govern a whole crew
        gov.watch(crew)
    """

    def govern(self, fn: F) -> F:
        """Decorator that gates and reports every call to *fn*.

        Before the wrapped function runs, ``check()`` is called.  If the
        gate returns *blocked*, a ``PermissionError`` is raised and the
        action is reported as denied.  Otherwise the function executes
        normally and the result is reported as allowed.
        """

        @functools.wraps(fn)
        def wrapper(*args: Any, **kwargs: Any) -> Any:
            action = fn.__qualname__
            verdict = self.check(action)

            if verdict.get("blocked"):
                self._report(action, blocked=True, reason=verdict.get("reason", ""))
                raise PermissionError(
                    f"Glassbox blocked {action}: {verdict.get('reason', 'denied')}"
                )

            try:
                result = fn(*args, **kwargs)
            except Exception:
                self._report(action, blocked=False, reason="execution error")
                raise

            self._report(action, blocked=False)
            return result

        return wrapper  # type: ignore[return-value]

    def watch(self, crew: Any) -> None:
        """Hook into a CrewAI ``Crew`` to report every task execution.

        CrewAI exposes ``step_callback`` and ``task_callback`` on the
        ``Crew`` object.  This method installs lightweight callbacks that
        report each step/task to Glassbox without interfering with
        normal execution.
        """
        original_step_cb = getattr(crew, "step_callback", None)
        original_task_cb = getattr(crew, "task_callback", None)

        def _step_callback(step_output: Any) -> None:
            action = str(getattr(step_output, "tool", "unknown_tool"))
            tool_input = str(getattr(step_output, "tool_input", ""))
            verdict = self.check(action, target=tool_input)
            blocked = verdict.get("blocked", False)
            self._report(
                action,
                blocked=blocked,
                reason=verdict.get("reason", ""),
                target=tool_input,
            )
            if original_step_cb:
                original_step_cb(step_output)

        def _task_callback(task_output: Any) -> None:
            description = str(getattr(task_output, "description", "task"))
            self._report(f"task:{description[:120]}", blocked=False)
            if original_task_cb:
                original_task_cb(task_output)

        crew.step_callback = _step_callback
        crew.task_callback = _task_callback


# =====================================================================
# LangGraph
# =====================================================================

class LangGraphGovernance(_BaseGovernance):
    """Governance node for LangGraph workflows.

    Usage::

        from glassbox.middleware import LangGraphGovernance
        gov = LangGraphGovernance("http://localhost:3120", "gbx_key")

        # Add as a node in your graph
        graph.add_node("governance", gov.node)

        # Then wire edges so traffic flows through it:
        graph.add_edge("agent", "governance")
        graph.add_edge("governance", "tools")
    """

    def node(self, state: dict) -> dict:
        """LangGraph-compatible node function.

        Inspects ``state["action"]`` (a string describing what the agent
        wants to do) and calls the Glassbox gate.  Returns a copy of *state*
        with a ``"governed"`` key added::

            {
                "governed": {
                    "decision": "allow",   # or "deny"
                    "blocked": False,
                    "reason": "...",
                }
            }

        If ``state`` has no ``"action"`` key the node is a no-op and
        passes the state through with ``governed.decision = "allow"``.
        """
        action = state.get("action", "")
        target = state.get("target", "")

        if not action:
            return {
                **state,
                "governed": {
                    "decision": "allow",
                    "blocked": False,
                    "reason": "no action specified",
                },
            }

        verdict = self.check(str(action), target=str(target))
        blocked = verdict.get("blocked", False)

        self._report(
            str(action),
            blocked=blocked,
            reason=verdict.get("reason", ""),
            target=str(target),
        )

        return {
            **state,
            "governed": {
                "decision": verdict.get("decision", "allow"),
                "blocked": blocked,
                "reason": verdict.get("reason", ""),
            },
        }


# =====================================================================
# AutoGen
# =====================================================================

class AutoGenGovernance(_BaseGovernance):
    """Governance hook for AutoGen agents.

    Usage::

        from glassbox.middleware import AutoGenGovernance
        gov = AutoGenGovernance("http://localhost:3120", "gbx_key")
        gov.register(agent)
    """

    def register(self, agent: Any) -> None:
        """Hook into an AutoGen agent's message processing.

        AutoGen agents expose ``register_hook(hookable_method, hook)``
        for intercepting message processing.  This method installs a
        hook on ``process_last_received_message`` that reports every
        incoming message to Glassbox and blocks it if the gate denies
        the action.

        For agents that lack ``register_hook`` (e.g. newer AutoGen
        versions), this falls back to wrapping ``generate_reply``
        directly.
        """
        agent_name = getattr(agent, "name", None) or self.agent_name

        if hasattr(agent, "register_hook"):
            # AutoGen v0.2+ hook system
            def _hook(message: Any) -> Any:
                content = self._extract_content(message)
                verdict = self.check(f"message:{content[:200]}")
                blocked = verdict.get("blocked", False)
                self._report(
                    f"message:{content[:200]}",
                    blocked=blocked,
                    reason=verdict.get("reason", ""),
                    agent=agent_name,
                )
                if blocked:
                    return (
                        f"[BLOCKED by Glassbox] {verdict.get('reason', 'denied')}"
                    )
                return message

            agent.register_hook(
                hookable_method="process_last_received_message",
                hook=_hook,
            )
        elif hasattr(agent, "generate_reply"):
            # Fallback: wrap generate_reply
            original_generate = agent.generate_reply

            @functools.wraps(original_generate)
            def _governed_generate(*args: Any, **kwargs: Any) -> Any:
                # Try to extract the latest message from the arguments
                messages = kwargs.get("messages") or (args[0] if args else [])
                if messages and isinstance(messages, list) and messages:
                    content = self._extract_content(messages[-1])
                else:
                    content = "unknown"

                verdict = self.check(f"reply:{content[:200]}")
                blocked = verdict.get("blocked", False)
                self._report(
                    f"reply:{content[:200]}",
                    blocked=blocked,
                    reason=verdict.get("reason", ""),
                    agent=agent_name,
                )

                if blocked:
                    return (
                        f"[BLOCKED by Glassbox] {verdict.get('reason', 'denied')}"
                    )
                return original_generate(*args, **kwargs)

            agent.generate_reply = _governed_generate
        else:
            logger.warning(
                "AutoGen agent %r has no hookable method — governance not installed",
                agent_name,
            )

    @staticmethod
    def _extract_content(message: Any) -> str:
        """Pull a string summary out of an AutoGen message."""
        if isinstance(message, str):
            return message
        if isinstance(message, dict):
            return str(message.get("content", message))
        return str(message)

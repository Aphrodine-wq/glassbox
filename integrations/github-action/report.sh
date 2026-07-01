#!/usr/bin/env bash
# report.sh — Post a governance decision to the Glassbox ingest API.
#
# Expected environment (set by action.yml):
#   GLASSBOX_URL       Base URL of the Glassbox server
#   GLASSBOX_API_KEY   API key for authentication
#   GLASSBOX_AGENT     Agent name (default: github-actions)
#   GLASSBOX_MODE      Governance mode: enforce | shadow
#   GLASSBOX_ACTION    Action string (optional override)
#   GLASSBOX_TARGET    Target string (optional override)
#
# GitHub-provided context:
#   GITHUB_REPOSITORY, GITHUB_WORKFLOW, GITHUB_RUN_ID, GITHUB_ACTOR,
#   GITHUB_REF, GITHUB_SHA, GITHUB_EVENT_NAME

set -euo pipefail

# ── Validate required inputs ────────────────────────────────────────
if [ -z "${GLASSBOX_URL:-}" ]; then
  echo "::error::glassbox-url is required"
  exit 1
fi

if [ -z "${GLASSBOX_API_KEY:-}" ]; then
  echo "::error::api-key is required"
  exit 1
fi

# ── Build the action string ─────────────────────────────────────────
# If the caller supplied an explicit action, use it. Otherwise, derive
# one from the GitHub Actions context so the decision log is useful.
if [ -n "${GLASSBOX_ACTION:-}" ]; then
  ACTION="${GLASSBOX_ACTION}"
else
  ACTION="${GITHUB_EVENT_NAME:-unknown}:${GITHUB_WORKFLOW:-unknown} (${GITHUB_REPOSITORY:-unknown}#${GITHUB_RUN_ID:-0} by ${GITHUB_ACTOR:-unknown})"
fi

# ── Build the target string ─────────────────────────────────────────
if [ -n "${GLASSBOX_TARGET:-}" ]; then
  TARGET="${GLASSBOX_TARGET}"
else
  TARGET="${GITHUB_REPOSITORY:-unknown}"
fi

AGENT="${GLASSBOX_AGENT:-github-actions}"
MODE="${GLASSBOX_MODE:-enforce}"

# ── Construct the JSON payload ──────────────────────────────────────
# Uses the same schema as `glassbox gate-json`:
#   { action, target, agent, mode }
# Plus a `context` block with full CI metadata for traceability.
PAYLOAD=$(cat <<EOF
{
  "action": $(echo -n "$ACTION" | jq -Rs .),
  "target": $(echo -n "$TARGET" | jq -Rs .),
  "agent":  $(echo -n "$AGENT"  | jq -Rs .),
  "mode":   $(echo -n "$MODE"   | jq -Rs .),
  "context": {
    "repository":  "${GITHUB_REPOSITORY:-}",
    "workflow":    "${GITHUB_WORKFLOW:-}",
    "run_id":      "${GITHUB_RUN_ID:-}",
    "actor":       "${GITHUB_ACTOR:-}",
    "ref":         "${GITHUB_REF:-}",
    "sha":         "${GITHUB_SHA:-}",
    "event":       "${GITHUB_EVENT_NAME:-}"
  }
}
EOF
)

# ── POST to the Glassbox ingest endpoint ────────────────────────────
INGEST_URL="${GLASSBOX_URL%/}/api/v1/ingest"

echo "Reporting decision to Glassbox: ${INGEST_URL}"
echo "  agent:  ${AGENT}"
echo "  mode:   ${MODE}"
echo "  action: ${ACTION}"

HTTP_CODE=$(curl -s -o /tmp/glassbox_response.json -w "%{http_code}" \
  -X POST "${INGEST_URL}" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer ${GLASSBOX_API_KEY}" \
  -d "${PAYLOAD}")

RESPONSE=$(cat /tmp/glassbox_response.json)

echo "Glassbox response (HTTP ${HTTP_CODE}):"
echo "${RESPONSE}" | jq . 2>/dev/null || echo "${RESPONSE}"

# ── Interpret the response ──────────────────────────────────────────
if [ "${HTTP_CODE}" -lt 200 ] || [ "${HTTP_CODE}" -ge 300 ]; then
  echo "::error::Glassbox ingest failed with HTTP ${HTTP_CODE}"
  exit 1
fi

# In enforce mode, a blocked decision should fail the workflow step.
BLOCKED=$(echo "${RESPONSE}" | jq -r '.blocked // false')
DECISION=$(echo "${RESPONSE}" | jq -r '.decision // "unknown"')

echo "decision=${DECISION}" >> "${GITHUB_OUTPUT:-/dev/null}"
echo "blocked=${BLOCKED}"   >> "${GITHUB_OUTPUT:-/dev/null}"

if [ "${MODE}" = "enforce" ] && [ "${BLOCKED}" = "true" ]; then
  REASON=$(echo "${RESPONSE}" | jq -r '.reason // "no reason given"')
  echo "::error::Glassbox governance blocked this action: ${REASON}"
  exit 1
fi

echo "Glassbox decision: ${DECISION}"

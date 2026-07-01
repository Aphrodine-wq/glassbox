/**
 * Glassbox JS/TS SDK — govern AI agents with one line of code.
 *
 * Usage:
 *   import { Glassbox } from "@glassbox/sdk";
 *
 *   const gb = new Glassbox("http://localhost:3120", "gbx_your_api_key");
 *
 *   // Report a governed decision
 *   await gb.report({ agent: "my-agent", action: "git push" });
 *
 *   // Report a blocked action
 *   await gb.block("my-agent", "rm -rf /", "safety rail: forbidden");
 *
 *   // Track costs
 *   await gb.cost({ agent: "my-agent", tokensIn: 15000, tokensOut: 3000, costUsd: 0.82 });
 *
 *   // Fire-and-forget (non-blocking)
 *   gb.reportAsync({ agent: "my-agent", action: "npm test" });
 */

export interface Decision {
  agent: string;
  action: string;
  blocked?: boolean;
  decision?: string;
  reason?: string;
  target?: string;
  mode?: string;
  provenanceId?: string;
  verdicts?: Verdict[];
  t?: number;
}

export interface Verdict {
  rail: string;
  refused: boolean;
  reason: string;
  policy: string;
}

export interface CostEvent {
  agent: string;
  tokensIn?: number;
  tokensOut?: number;
  costUsd?: number;
  model?: string;
  t?: number;
}

export interface IngestResponse {
  ingested: number;
}

export class GlassboxError extends Error {
  status: number;
  body: string;
  constructor(status: number, body: string) {
    super(`Glassbox API error ${status}: ${body}`);
    this.status = status;
    this.body = body;
  }
}

export class Glassbox {
  private url: string;
  private apiKey: string;
  private timeout: number;

  constructor(url: string, apiKey: string, timeout = 10000) {
    this.url = url.replace(/\/+$/, "");
    this.apiKey = apiKey;
    this.timeout = timeout;
  }

  /** Report a single governed decision. */
  async report(d: Decision): Promise<IngestResponse> {
    return this.post("/api/ingest/decision", toWire(d));
  }

  /** Report a decision without awaiting the response. */
  reportAsync(d: Decision): void {
    this.post("/api/ingest/decision", toWire(d)).catch(() => {});
  }

  /** Report multiple decisions in a single request. */
  async reportBatch(decisions: Decision[]): Promise<IngestResponse> {
    return this.post("/api/ingest/decision", decisions.map(toWire));
  }

  /** Report a cost/token event. */
  async cost(c: CostEvent): Promise<IngestResponse> {
    return this.post("/api/ingest/cost", costToWire(c));
  }

  /** Report a cost event without awaiting. */
  costAsync(c: CostEvent): void {
    this.post("/api/ingest/cost", costToWire(c)).catch(() => {});
  }

  /** Report multiple cost events. */
  async costBatch(events: CostEvent[]): Promise<IngestResponse> {
    return this.post("/api/ingest/cost", events.map(costToWire));
  }

  /** Shorthand: report an allowed action. */
  async allow(agent: string, action: string, extra?: Partial<Decision>): Promise<IngestResponse> {
    return this.report({
      agent,
      action,
      blocked: false,
      decision: "allow",
      reason: "all rails clean",
      ...extra,
    });
  }

  /** Shorthand: report a blocked action. */
  async block(agent: string, action: string, reason: string, extra?: Partial<Decision>): Promise<IngestResponse> {
    return this.report({
      agent,
      action,
      blocked: true,
      decision: "deny",
      reason,
      ...extra,
    });
  }

  /**
   * Track costs over a block of work.
   *
   *   const tracker = gb.track("my-agent", "claude-sonnet-4");
   *   // ... do work, call tracker.add(tokensIn, tokensOut, costUsd) ...
   *   await tracker.flush();
   */
  track(agent: string, model = ""): CostTracker {
    return new CostTracker(this, agent, model);
  }

  // ── Internal ───────────────────────────────────────────────────────

  private async post(path: string, payload: unknown): Promise<IngestResponse> {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), this.timeout);

    try {
      const res = await fetch(this.url + path, {
        method: "POST",
        headers: {
          Authorization: `Bearer ${this.apiKey}`,
          "Content-Type": "application/json",
        },
        body: JSON.stringify(payload),
        signal: controller.signal,
      });

      if (!res.ok) {
        const body = await res.text();
        throw new GlassboxError(res.status, body);
      }

      return (await res.json()) as IngestResponse;
    } finally {
      clearTimeout(timer);
    }
  }
}

export class CostTracker {
  private client: Glassbox;
  private agent: string;
  private model: string;
  private startTime: number;
  tokensIn = 0;
  tokensOut = 0;
  costUsd = 0;

  constructor(client: Glassbox, agent: string, model: string) {
    this.client = client;
    this.agent = agent;
    this.model = model;
    this.startTime = Date.now();
  }

  /** Accumulate usage. */
  add(tokensIn = 0, tokensOut = 0, costUsd = 0): void {
    this.tokensIn += tokensIn;
    this.tokensOut += tokensOut;
    this.costUsd += costUsd;
  }

  /** Send the accumulated cost event. */
  async flush(): Promise<IngestResponse> {
    return this.client.cost({
      agent: this.agent,
      tokensIn: this.tokensIn,
      tokensOut: this.tokensOut,
      costUsd: this.costUsd,
      model: this.model,
      t: this.startTime,
    });
  }

  /** Send without awaiting. */
  flushAsync(): void {
    this.client.costAsync({
      agent: this.agent,
      tokensIn: this.tokensIn,
      tokensOut: this.tokensOut,
      costUsd: this.costUsd,
      model: this.model,
      t: this.startTime,
    });
  }
}

// ── Wire format helpers ──────────────────────────────────────────────────

function toWire(d: Decision): Record<string, unknown> {
  return {
    agent: d.agent,
    action: d.action,
    blocked: d.blocked ?? false,
    decision: d.decision ?? "allow",
    reason: d.reason ?? "all rails clean",
    target: d.target ?? "",
    mode: d.mode ?? "enforce",
    provenance_id: d.provenanceId ?? "",
    verdicts: d.verdicts ?? [],
    t: d.t ?? Date.now(),
  };
}

function costToWire(c: CostEvent): Record<string, unknown> {
  return {
    agent: c.agent,
    tokens_in: c.tokensIn ?? 0,
    tokens_out: c.tokensOut ?? 0,
    cost_usd: c.costUsd ?? 0,
    model: c.model ?? "",
    t: c.t ?? Date.now(),
  };
}

import * as vscode from "vscode";
import * as http from "http";
import * as https from "https";
import { URL } from "url";

export interface Agent {
  id: string;
  name: string;
  status: string;
  model?: string;
  lastSeen?: string;
}

export interface Decision {
  id: string;
  agentId: string;
  agentName: string;
  action: string;
  outcome: string;
  timestamp: string;
  reason?: string;
}

export interface Overview {
  totalBudget: number;
  spent: number;
  remaining: number;
  currency: string;
  periodStart?: string;
  periodEnd?: string;
}

function getConfig(): { serverUrl: string; apiKey: string } {
  const config = vscode.workspace.getConfiguration("glassbox");
  return {
    serverUrl: config.get<string>("serverUrl", "http://localhost:3120"),
    apiKey: config.get<string>("apiKey", ""),
  };
}

function request<T>(path: string): Promise<T> {
  const { serverUrl, apiKey } = getConfig();
  const url = new URL(path, serverUrl);

  return new Promise((resolve, reject) => {
    const transport = url.protocol === "https:" ? https : http;

    const options: http.RequestOptions = {
      hostname: url.hostname,
      port: url.port,
      path: url.pathname + url.search,
      method: "GET",
      headers: {
        Accept: "application/json",
        ...(apiKey ? { Authorization: `Bearer ${apiKey}` } : {}),
      },
    };

    const req = transport.request(options, (res) => {
      let body = "";
      res.on("data", (chunk: Buffer) => {
        body += chunk.toString();
      });
      res.on("end", () => {
        if (res.statusCode && res.statusCode >= 200 && res.statusCode < 300) {
          try {
            resolve(JSON.parse(body) as T);
          } catch {
            reject(new Error(`Invalid JSON response from ${path}`));
          }
        } else {
          reject(
            new Error(`HTTP ${res.statusCode} from ${path}: ${body.slice(0, 200)}`)
          );
        }
      });
    });

    req.on("error", (err) => {
      reject(new Error(`Request to ${path} failed: ${err.message}`));
    });

    req.setTimeout(10_000, () => {
      req.destroy();
      reject(new Error(`Request to ${path} timed out`));
    });

    req.end();
  });
}

export function fetchOverview(): Promise<Overview> {
  return request<Overview>("/api/overview");
}

export function fetchAgents(): Promise<Agent[]> {
  return request<Agent[]>("/api/agents");
}

export function fetchDecisions(limit: number = 20): Promise<Decision[]> {
  return request<Decision[]>(`/api/decisions?limit=${limit}`);
}

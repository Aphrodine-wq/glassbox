import * as vscode from "vscode";
import {
  fetchAgents,
  fetchDecisions,
  fetchOverview,
  Agent,
  Decision,
  Overview,
} from "./api";

// ---------- Tree Data Providers ----------

class AgentsProvider implements vscode.TreeDataProvider<vscode.TreeItem> {
  private _onDidChangeTreeData = new vscode.EventEmitter<void>();
  readonly onDidChangeTreeData = this._onDidChangeTreeData.event;

  private agents: Agent[] = [];

  refresh(): void {
    this._onDidChangeTreeData.fire();
  }

  getAgents(): Agent[] {
    return this.agents;
  }

  async getChildren(): Promise<vscode.TreeItem[]> {
    try {
      this.agents = await fetchAgents();
    } catch {
      this.agents = [];
      return [new vscode.TreeItem("Unable to reach Glassbox server")];
    }

    if (this.agents.length === 0) {
      return [new vscode.TreeItem("No agents registered")];
    }

    return this.agents.map((agent) => {
      const item = new vscode.TreeItem(
        agent.name,
        vscode.TreeItemCollapsibleState.None
      );
      item.description = agent.status;
      item.tooltip = `${agent.name} (${agent.id})\nStatus: ${agent.status}${
        agent.model ? `\nModel: ${agent.model}` : ""
      }${agent.lastSeen ? `\nLast seen: ${agent.lastSeen}` : ""}`;

      switch (agent.status) {
        case "running":
          item.iconPath = new vscode.ThemeIcon(
            "pass",
            new vscode.ThemeColor("testing.iconPassed")
          );
          break;
        case "blocked":
          item.iconPath = new vscode.ThemeIcon(
            "error",
            new vscode.ThemeColor("testing.iconFailed")
          );
          break;
        case "idle":
          item.iconPath = new vscode.ThemeIcon("circle-outline");
          break;
        default:
          item.iconPath = new vscode.ThemeIcon("question");
      }

      return item;
    });
  }

  getTreeItem(element: vscode.TreeItem): vscode.TreeItem {
    return element;
  }
}

class DecisionsProvider implements vscode.TreeDataProvider<vscode.TreeItem> {
  private _onDidChangeTreeData = new vscode.EventEmitter<void>();
  readonly onDidChangeTreeData = this._onDidChangeTreeData.event;

  refresh(): void {
    this._onDidChangeTreeData.fire();
  }

  async getChildren(): Promise<vscode.TreeItem[]> {
    let decisions: Decision[];
    try {
      decisions = await fetchDecisions(20);
    } catch {
      return [new vscode.TreeItem("Unable to reach Glassbox server")];
    }

    if (decisions.length === 0) {
      return [new vscode.TreeItem("No recent decisions")];
    }

    return decisions.map((d) => {
      const label = `${d.agentName}: ${d.action}`;
      const item = new vscode.TreeItem(label, vscode.TreeItemCollapsibleState.None);
      item.description = d.outcome;
      item.tooltip = `Agent: ${d.agentName}\nAction: ${d.action}\nOutcome: ${d.outcome}\nTime: ${d.timestamp}${
        d.reason ? `\nReason: ${d.reason}` : ""
      }`;

      switch (d.outcome) {
        case "approved":
          item.iconPath = new vscode.ThemeIcon(
            "check",
            new vscode.ThemeColor("testing.iconPassed")
          );
          break;
        case "denied":
          item.iconPath = new vscode.ThemeIcon(
            "close",
            new vscode.ThemeColor("testing.iconFailed")
          );
          break;
        case "escalated":
          item.iconPath = new vscode.ThemeIcon(
            "warning",
            new vscode.ThemeColor("editorWarning.foreground")
          );
          break;
        default:
          item.iconPath = new vscode.ThemeIcon("circle-outline");
      }

      return item;
    });
  }

  getTreeItem(element: vscode.TreeItem): vscode.TreeItem {
    return element;
  }
}

class BudgetProvider implements vscode.TreeDataProvider<vscode.TreeItem> {
  private _onDidChangeTreeData = new vscode.EventEmitter<void>();
  readonly onDidChangeTreeData = this._onDidChangeTreeData.event;

  refresh(): void {
    this._onDidChangeTreeData.fire();
  }

  async getChildren(): Promise<vscode.TreeItem[]> {
    let overview: Overview;
    try {
      overview = await fetchOverview();
    } catch {
      return [new vscode.TreeItem("Unable to reach Glassbox server")];
    }

    const pct =
      overview.totalBudget > 0
        ? ((overview.spent / overview.totalBudget) * 100).toFixed(1)
        : "0.0";

    const items: vscode.TreeItem[] = [];

    const totalItem = new vscode.TreeItem(
      `Total Budget: ${overview.totalBudget} ${overview.currency}`,
      vscode.TreeItemCollapsibleState.None
    );
    totalItem.iconPath = new vscode.ThemeIcon("credit-card");
    items.push(totalItem);

    const spentItem = new vscode.TreeItem(
      `Spent: ${overview.spent} ${overview.currency} (${pct}%)`,
      vscode.TreeItemCollapsibleState.None
    );
    spentItem.iconPath = new vscode.ThemeIcon("graph");
    items.push(spentItem);

    const remainItem = new vscode.TreeItem(
      `Remaining: ${overview.remaining} ${overview.currency}`,
      vscode.TreeItemCollapsibleState.None
    );
    remainItem.iconPath = new vscode.ThemeIcon("wallet");
    items.push(remainItem);

    if (overview.periodStart && overview.periodEnd) {
      const periodItem = new vscode.TreeItem(
        `Period: ${overview.periodStart} - ${overview.periodEnd}`,
        vscode.TreeItemCollapsibleState.None
      );
      periodItem.iconPath = new vscode.ThemeIcon("calendar");
      items.push(periodItem);
    }

    return items;
  }

  getTreeItem(element: vscode.TreeItem): vscode.TreeItem {
    return element;
  }
}

// ---------- Status Bar ----------

function updateStatusBar(
  statusBarItem: vscode.StatusBarItem,
  agents: Agent[]
): void {
  const total = agents.length;
  const blocked = agents.filter((a) => a.status === "blocked").length;

  if (total === 0) {
    statusBarItem.text = "$(shield) Glassbox: no agents";
  } else {
    statusBarItem.text = `$(shield) Glassbox: ${total} agent${total !== 1 ? "s" : ""}${
      blocked > 0 ? `, ${blocked} blocked` : ""
    }`;
  }

  if (blocked > 0) {
    statusBarItem.backgroundColor = new vscode.ThemeColor(
      "statusBarItem.warningBackground"
    );
  } else {
    statusBarItem.backgroundColor = undefined;
  }

  statusBarItem.show();
}

// ---------- Activation ----------

export function activate(context: vscode.ExtensionContext): void {
  const agentsProvider = new AgentsProvider();
  const decisionsProvider = new DecisionsProvider();
  const budgetProvider = new BudgetProvider();

  context.subscriptions.push(
    vscode.window.registerTreeDataProvider("agents", agentsProvider),
    vscode.window.registerTreeDataProvider("decisions", decisionsProvider),
    vscode.window.registerTreeDataProvider("budget", budgetProvider)
  );

  // Status bar
  const statusBarItem = vscode.window.createStatusBarItem(
    vscode.StatusBarAlignment.Left,
    50
  );
  statusBarItem.command = "glassbox.refresh";
  statusBarItem.tooltip = "Click to refresh Glassbox status";
  statusBarItem.text = "$(shield) Glassbox: loading...";
  statusBarItem.show();
  context.subscriptions.push(statusBarItem);

  // Refresh helper
  async function refreshAll(): Promise<void> {
    agentsProvider.refresh();
    decisionsProvider.refresh();
    budgetProvider.refresh();

    try {
      const agents = await fetchAgents();
      updateStatusBar(statusBarItem, agents);
    } catch {
      statusBarItem.text = "$(shield) Glassbox: offline";
      statusBarItem.backgroundColor = new vscode.ThemeColor(
        "statusBarItem.errorBackground"
      );
    }
  }

  // Commands
  context.subscriptions.push(
    vscode.commands.registerCommand("glassbox.refresh", () => {
      refreshAll();
    })
  );

  context.subscriptions.push(
    vscode.commands.registerCommand("glassbox.exportReport", async () => {
      try {
        const [agents, decisions, overview] = await Promise.all([
          fetchAgents(),
          fetchDecisions(50),
          fetchOverview(),
        ]);

        const report = {
          exportedAt: new Date().toISOString(),
          overview,
          agents,
          decisions,
        };

        const uri = await vscode.window.showSaveDialog({
          defaultUri: vscode.Uri.file("glassbox-report.json"),
          filters: { JSON: ["json"] },
        });

        if (uri) {
          const content = Buffer.from(JSON.stringify(report, null, 2), "utf-8");
          await vscode.workspace.fs.writeFile(uri, content);
          vscode.window.showInformationMessage(
            `Glassbox report exported to ${uri.fsPath}`
          );
        }
      } catch (err) {
        vscode.window.showErrorMessage(
          `Failed to export report: ${err instanceof Error ? err.message : String(err)}`
        );
      }
    })
  );

  context.subscriptions.push(
    vscode.commands.registerCommand("glassbox.toggleMode", async () => {
      const config = vscode.workspace.getConfiguration("glassbox");
      const currentUrl = config.get<string>("serverUrl", "http://localhost:3120");

      const input = await vscode.window.showInputBox({
        prompt: "Enter Glassbox server URL",
        value: currentUrl,
        placeHolder: "http://localhost:3120",
      });

      if (input !== undefined) {
        await config.update("serverUrl", input, vscode.ConfigurationTarget.Global);
        vscode.window.showInformationMessage(`Glassbox server set to ${input}`);
        refreshAll();
      }
    })
  );

  // Auto-refresh every 10 seconds
  const refreshInterval = setInterval(() => {
    refreshAll();
  }, 10_000);

  context.subscriptions.push({
    dispose: () => clearInterval(refreshInterval),
  });

  // Initial refresh
  refreshAll();
}

export function deactivate(): void {
  // Cleanup handled by context.subscriptions disposal
}

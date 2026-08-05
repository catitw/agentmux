// agentmux omp/pi extension — installed by `agentmux --install-hooks`.
//
// Runs inside the omp (pi-coding-agent) process and reports lifecycle
// states to the agentmux loopback server using the agent's own pid
// (process.pid). No-op silently when agentmux is not running (no port file).
// State mapping mirrors the claude hook: working / idle / blocked / clear.
// AGENTMUX_EXTENSION_ID=omp
// @ts-nocheck

import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";

function portFilePath() {
  if (process.platform === "win32") {
    const appdata = process.env.APPDATA;
    return appdata ? path.join(appdata, "agentmux", "agentmux.port") : undefined;
  }
  const home = os.homedir();
  return home ? path.join(home, ".config", "agentmux", "agentmux.port") : undefined;
}

function readPort() {
  const file = portFilePath();
  if (!file) return undefined;
  try {
    const port = parseInt(fs.readFileSync(file, "utf8").trim(), 10);
    return Number.isFinite(port) && port > 0 && port < 65536 ? port : undefined;
  } catch {
    return undefined;
  }
}

function report(state, message) {
  const port = readPort();
  if (port === undefined) return;
  const body = JSON.stringify({ pid: process.pid, agent: "omp", state, message });
  const req = http.request(
    {
      host: "127.0.0.1",
      port,
      path: "/report",
      method: "POST",
      headers: { "Content-Type": "application/json" },
    },
    (res) => {
      res.resume();
    },
  );
  req.setTimeout(2000, () => req.destroy());
  req.on("error", () => {});
  req.end(body);
}

export default function (pi) {
  let agentActive = false;
  let blockedCount = 0;
  let blockedMessage;
  let lastState;

  function desiredState() {
    if (blockedCount > 0) {
      return { state: "blocked", message: blockedMessage };
    }
    if (agentActive) {
      return { state: "working", message: undefined };
    }
    return { state: "idle", message: undefined };
  }

  function publish() {
    const next = desiredState();
    if (next.state === lastState) {
      return;
    }
    lastState = next.state;
    report(next.state, next.message);
  }

  pi.on("session_start", (_event, ctx) => {
    agentActive = ctx?.isIdle?.() === false;
    publish();
  });

  pi.on("session_switch", () => {
    blockedCount = 0;
    blockedMessage = undefined;
    publish();
  });

  pi.on("agent_start", () => {
    agentActive = true;
    publish();
  });

  pi.on("agent_end", () => {
    agentActive = false;
    publish();
  });

  pi.on("tool_approval_requested", (event) => {
    blockedCount += 1;
    blockedMessage = event?.reason || `${event?.toolName || "Tool"} approval`;
    publish();
  });

  pi.on("tool_approval_resolved", () => {
    blockedCount = Math.max(0, blockedCount - 1);
    if (blockedCount === 0) {
      blockedMessage = undefined;
    }
    publish();
  });

  pi.on("tool_execution_start", (event) => {
    if (event?.toolName !== "ask") {
      return;
    }
    blockedCount += 1;
    blockedMessage = "waiting for user input";
    publish();
  });

  pi.on("tool_execution_end", (event) => {
    if (event?.toolName !== "ask") {
      return;
    }
    blockedCount = Math.max(0, blockedCount - 1);
    if (blockedCount === 0) {
      blockedMessage = undefined;
    }
    publish();
  });

  pi.on("session_shutdown", () => {
    report("clear");
  });
}

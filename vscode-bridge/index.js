#!/usr/bin/env node
/**
 * VS Code Bridge - Host-side service for opening files in VS Code
 *
 * Runs on the Windows host, receives HTTP requests from container agents,
 * translates container paths to host paths, and opens files in VS Code.
 *
 * Endpoints:
 *   GET  /health  - Health check
 *   POST /open    - Open file in VS Code
 */

const http = require("http");
const { execFile } = require("child_process");
const path = require("path");
const fs = require("fs");

const PORT = parseInt(process.env.VSCODE_BRIDGE_PORT || "3101");
const HOST = process.env.VSCODE_BRIDGE_HOST || "0.0.0.0";

// Map container agent IDs to host workspace paths
// Uses CLAW_WORKSPACES_DIR env var or defaults to standard location
const WORKSPACES_BASE = process.env.CLAW_WORKSPACES_DIR ||
  path.join(process.env.USERPROFILE || process.env.HOME || "C:\\Users\\asafe", ".claw", "workspaces");

function getHostPath(agentId, containerPath) {
  // Container paths start with /workspace, map to host workspace
  const hostBase = path.join(WORKSPACES_BASE, agentId);

  if (containerPath.startsWith("/workspace")) {
    // Remove /workspace prefix and join with host base
    const relativePath = containerPath.slice("/workspace".length);
    const fullPath = path.join(hostBase, relativePath);

    // Prevent directory traversal - ensure resolved path is within workspace
    const normalizedPath = path.normalize(fullPath);
    const normalizedBase = path.normalize(hostBase);

    // Check path is within workspace - must be exact match OR start with base + separator
    // This prevents prefix matching (e.g., "agent1" matching "agent10")
    if (normalizedPath !== normalizedBase &&
        !normalizedPath.startsWith(normalizedBase + path.sep)) {
      throw new Error("Invalid path: directory traversal detected");
    }

    return normalizedPath;
  }

  // If not a workspace path, return as-is (might be absolute host path)
  return containerPath;
}

function openInVSCode(hostPath, line, column, callback) {
  // Validate line and column are positive integers if provided
  if (line !== undefined) {
    const lineNum = parseInt(line, 10);
    if (!Number.isInteger(lineNum) || lineNum < 1) {
      return callback({ success: false, error: "Invalid line number" });
    }
    line = lineNum;
  }

  if (column !== undefined) {
    const colNum = parseInt(column, 10);
    if (!Number.isInteger(colNum) || colNum < 1) {
      return callback({ success: false, error: "Invalid column number" });
    }
    column = colNum;
  }

  // Build arguments array for execFile (no shell injection)
  const args = [];

  if (line) {
    const gotoTarget = column ? `${hostPath}:${line}:${column}` : `${hostPath}:${line}`;
    args.push("--goto", gotoTarget);
  } else {
    args.push(hostPath);
  }

  console.log(`[vscode-bridge] Executing: code ${args.join(" ")}`);

  // Use execFile instead of exec - doesn't spawn a shell, safer
  execFile("code", args, (error, stdout, stderr) => {
    if (error) {
      console.error(`[vscode-bridge] Error: ${error.message}`);
      callback({ success: false, error: error.message });
    } else {
      console.log(`[vscode-bridge] Opened: ${hostPath}`);
      callback({ success: true, path: hostPath, line, column });
    }
  });
}

const server = http.createServer((req, res) => {
  const sendJson = (statusCode, data) => {
    res.writeHead(statusCode, { "Content-Type": "application/json" });
    res.end(JSON.stringify(data));
  };

  // Health check endpoint
  if (req.method === "GET" && req.url === "/health") {
    return sendJson(200, {
      status: "ok",
      service: "vscode-bridge",
      version: "1.0.0",
      timestamp: new Date().toISOString(),
      workspacesBase: WORKSPACES_BASE
    });
  }

  // Open file endpoint
  if (req.method === "POST" && req.url === "/open") {
    let body = "";

    req.on("data", chunk => { body += chunk; });

    req.on("end", () => {
      try {
        const { agentId, path: filePath, line, column } = JSON.parse(body);

        if (!agentId) {
          return sendJson(400, { error: "agentId is required" });
        }
        if (!filePath) {
          return sendJson(400, { error: "path is required" });
        }

        const hostPath = getHostPath(agentId, filePath);

        openInVSCode(hostPath, line, column, (result) => {
          sendJson(result.success ? 200 : 500, result);
        });
      } catch (e) {
        console.error(`[vscode-bridge] Parse error: ${e.message}`);
        sendJson(400, { error: "Invalid JSON body" });
      }
    });

    return;
  }

  // Unknown endpoint
  sendJson(404, { error: "Not found", endpoints: ["GET /health", "POST /open"] });
});

server.listen(PORT, HOST, () => {
  console.log(`[vscode-bridge] VS Code Bridge v1.0.0`);
  console.log(`[vscode-bridge] Listening on http://${HOST}:${PORT}`);
  console.log(`[vscode-bridge] Workspaces base: ${WORKSPACES_BASE}`);
  console.log(`[vscode-bridge] Endpoints:`);
  console.log(`[vscode-bridge]   GET  /health  - Health check`);
  console.log(`[vscode-bridge]   POST /open    - Open file in VS Code`);
});

// Graceful shutdown
process.on("SIGTERM", () => {
  console.log("[vscode-bridge] Shutting down...");
  server.close(() => process.exit(0));
});

process.on("SIGINT", () => {
  console.log("[vscode-bridge] Interrupted, shutting down...");
  server.close(() => process.exit(0));
});

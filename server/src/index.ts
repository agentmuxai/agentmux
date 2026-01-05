import Fastify from "fastify";
import cors from "@fastify/cors";
import { MessageStore } from "./store.js";

const PORT = parseInt(process.env.PORT || "3100");
const HOST = process.env.HOST || "0.0.0.0";
const DB_PATH = process.env.DB_PATH || "/data/agentmux.db";

const store = new MessageStore(DB_PATH);
const app = Fastify({ logger: true });

await app.register(cors, { origin: true });

// Health & Stats
app.get("/api/health", async () => {
  return { status: "ok", version: "1.0.0", timestamp: new Date().toISOString() };
});

app.get("/api/stats", async () => {
  return await store.getStats();
});

// REST: Send message
app.post<{ Body: { to: string; message: string; priority?: string }; Headers: { "x-agent-id"?: string } }>(
  "/api/messages",
  async (request, reply) => {
    const agentId = request.headers["x-agent-id"];
    if (!agentId) return reply.status(400).send({ error: "X-Agent-ID header required" });
    const { to, message, priority = "normal" } = request.body;
    const msg = await store.sendMessage(agentId, to, message, priority);
    return { success: true, message_id: msg.id, from: agentId, to, delivered_at: msg.timestamp, priority };
  }
);

// REST: Read messages
app.get<{ Querystring: { unread_only?: string; limit?: string; mark_as_read?: string }; Headers: { "x-agent-id"?: string } }>(
  "/api/messages",
  async (request, reply) => {
    const agentId = request.headers["x-agent-id"];
    if (!agentId) return reply.status(400).send({ error: "X-Agent-ID header required" });
    const unreadOnly = request.query.unread_only !== "false";
    const limit = parseInt(request.query.limit || "10");
    const markAsRead = request.query.mark_as_read !== "false";
    const messages = await store.readMessages(agentId, unreadOnly, limit, markAsRead);
    return {
      agent_id: agentId,
      messages: messages.map(m => ({ id: m.id, from: m.from_agent, message: m.text, timestamp: m.timestamp, priority: m.priority, read: m.read })),
      count: messages.length
    };
  }
);

// REST: List agents
app.get("/api/agents", async () => {
  const agents = await store.listAgents();
  return { agents, total_count: agents.length };
});

// REST: Delete messages
app.delete<{ Body: { message_ids: string[] }; Headers: { "x-agent-id"?: string } }>(
  "/api/messages",
  async (request, reply) => {
    const agentId = request.headers["x-agent-id"];
    if (!agentId) return reply.status(400).send({ error: "X-Agent-ID header required" });
    const { message_ids } = request.body;
    const result = await store.deleteMessages(agentId, message_ids);
    return { ...result, deleted_count: result.deleted.length };
  }
);

// VS Code Bridge configuration
const VSCODE_BRIDGE_URL = process.env.VSCODE_BRIDGE_URL || "http://host.docker.internal:3101";

// MCP Tools Definition
const MCP_TOOLS = [
  { name: "send_message", description: "Send a message to another agent", inputSchema: { type: "object", properties: { to: { type: "string" }, message: { type: "string" }, priority: { type: "string", enum: ["low","normal","high","urgent"], default: "normal" } }, required: ["to","message"] } },
  { name: "read_messages", description: "Read messages sent to this agent", inputSchema: { type: "object", properties: { unread_only: { type: "boolean", default: true }, limit: { type: "number", default: 10 }, mark_as_read: { type: "boolean", default: true } } } },
  { name: "list_agents", description: "List all agents", inputSchema: { type: "object", properties: {} } },
  { name: "broadcast_message", description: "Send to all agents", inputSchema: { type: "object", properties: { message: { type: "string" }, priority: { type: "string", default: "normal" } }, required: ["message"] } },
  { name: "delete_messages", description: "Delete messages by ID", inputSchema: { type: "object", properties: { message_ids: { type: "array", items: { type: "string" } } }, required: ["message_ids"] } },
  { name: "open_vscode", description: "Open a file in VS Code on the host machine", inputSchema: { type: "object", properties: { path: { type: "string", description: "File path (container path like /workspace/src/file.ts)" }, line: { type: "number", description: "Line number to navigate to (optional)" }, column: { type: "number", description: "Column number to navigate to (optional)" } }, required: ["path"] } }
];

interface MCPRequest { jsonrpc: "2.0"; id: number | string; method: string; params?: Record<string, unknown>; }

// MCP endpoint info (for clients checking availability)
app.get("/mcp", async (request, reply) => {
  return {
    name: "agentmux-server",
    version: "1.0.0",
    protocol: "JSON-RPC over HTTP",
    transport: "POST only (SSE not implemented)",
    usage: "Send JSON-RPC 2.0 requests via POST with X-Agent-ID header"
  };
});

// MCP JSON-RPC endpoint
app.post<{ Body: MCPRequest; Headers: { "x-agent-id"?: string } }>("/mcp", async (request, reply) => {
  const agentId = request.headers["x-agent-id"] || "unknown";
  const { jsonrpc, id, method, params } = request.body;
  if (jsonrpc !== "2.0") return reply.status(400).send({ jsonrpc: "2.0", id, error: { code: -32600, message: "Invalid JSON-RPC" } });

  try {
    let result: unknown;
    switch (method) {
      case "initialize":
        result = { protocolVersion: "2024-11-05", capabilities: { tools: {} }, serverInfo: { name: "agentmux-server", version: "1.0.0" } };
        break;
      case "tools/list":
        result = { tools: MCP_TOOLS };
        break;
      case "tools/call": {
        const toolName = (params as any)?.name;
        const args = (params as any)?.arguments || {};
        switch (toolName) {
          case "send_message": {
            const msg = await store.sendMessage(agentId, args.to, args.message, args.priority || "normal");
            result = { content: [{ type: "text", text: JSON.stringify({ success: true, message_id: msg.id, from: agentId, to: args.to, delivered_at: msg.timestamp, priority: args.priority || "normal" }, null, 2) }] };
            break;
          }
          case "read_messages": {
            const messages = await store.readMessages(agentId, args.unread_only ?? true, args.limit ?? 10, args.mark_as_read ?? true);
            result = { content: [{ type: "text", text: JSON.stringify({ agent_id: agentId, messages: messages.map(m => ({ id: m.id, from: m.from_agent, message: m.text, timestamp: m.timestamp, priority: m.priority, read: m.read })), count: messages.length }, null, 2) }] };
            break;
          }
          case "list_agents": {
            const agents = await store.listAgents();
            result = { content: [{ type: "text", text: JSON.stringify({ current_agent: agentId, agents, total_count: agents.length }, null, 2) }] };
            break;
          }
          case "broadcast_message": {
            const msg = await store.sendMessage(agentId, "*", args.message, args.priority || "normal");
            result = { content: [{ type: "text", text: JSON.stringify({ success: true, message_id: msg.id, from: agentId, to: "all agents", delivered_at: msg.timestamp, broadcast: true }, null, 2) }] };
            break;
          }
          case "delete_messages": {
            const delResult = await store.deleteMessages(agentId, args.message_ids);
            result = { content: [{ type: "text", text: JSON.stringify({ ...delResult, deleted_count: delResult.deleted.length }, null, 2) }] };
            break;
          }
          case "open_vscode": {
            try {
              const resp = await fetch(`${VSCODE_BRIDGE_URL}/open`, {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify({ agentId, path: args.path, line: args.line, column: args.column })
              });
              const data = await resp.json();
              result = { content: [{ type: "text", text: JSON.stringify(data, null, 2) }] };
            } catch (err) {
              result = { content: [{ type: "text", text: JSON.stringify({ success: false, error: `Failed to connect to VS Code bridge: ${(err as Error).message}` }, null, 2) }] };
            }
            break;
          }
          default:
            throw new Error(`Unknown tool: ${toolName}`);
        }
        break;
      }
      default:
        return reply.status(400).send({ jsonrpc: "2.0", id, error: { code: -32601, message: `Method not found: ${method}` } });
    }
    return { jsonrpc: "2.0", id, result };
  } catch (error) {
    return { jsonrpc: "2.0", id, error: { code: -32000, message: (error as Error).message } };
  }
});

// Export app for Lambda handler
export { app };

// Startup (only if not running in Lambda)
if (!process.env.AWS_LAMBDA_FUNCTION_NAME) {
  try {
    await app.listen({ port: PORT, host: HOST });
    console.log(`AgentMux Server v1.0.0 listening on http://${HOST}:${PORT}`);
    console.log(`MCP endpoint: POST http://${HOST}:${PORT}/mcp`);
    console.log(`REST API: http://${HOST}:${PORT}/api/*`);
  } catch (err) {
    app.log.error(err);
    process.exit(1);
  }
}

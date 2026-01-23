import Fastify from "fastify";
import cors from "@fastify/cors";
import { MessageStore } from "./store.js";
import { SecretsManagerClient, GetSecretValueCommand } from "@aws-sdk/client-secrets-manager";

const PORT = parseInt(process.env.PORT || "3100");
const HOST = process.env.HOST || "0.0.0.0";

const store = new MessageStore();
const app = Fastify({ logger: true });

await app.register(cors, { origin: true });

// Authentication: Bearer token validation
let cachedApiKey: string | null = null;

async function getApiKey(): Promise<string> {
  if (cachedApiKey) return cachedApiKey;

  const client = new SecretsManagerClient({ region: process.env.AWS_REGION || "us-east-1" });
  const response = await client.send(new GetSecretValueCommand({ SecretId: "services/infra" }));
  const secrets = JSON.parse(response.SecretString || "{}");
  cachedApiKey = secrets["agentmux-api-key"];

  if (!cachedApiKey) {
    throw new Error("agentmux-api-key not found in services/infra");
  }

  return cachedApiKey;
}

/**
 * Normalize agent ID for case-insensitive matching.
 * All agent IDs are stored and compared in lowercase.
 */
function normalizeAgentId(agentId: string): string {
  return agentId.toLowerCase().trim();
}

// Auth middleware: Validate bearer token
app.addHook("onRequest", async (request, reply) => {
  // Skip auth for health check
  if (request.url === "/api/health") return;

  const authHeader = request.headers.authorization || "";
  const expectedKey = await getApiKey();

  if (authHeader !== `Bearer ${expectedKey}`) {
    reply.code(401).send({ error: "Unauthorized", message: "Invalid or missing bearer token" });
  }
});

// Health & Stats
app.get("/api/health", async () => {
  return { status: "ok", version: "1.1.0", timestamp: new Date().toISOString() };
});

app.get("/api/stats", async () => {
  return await store.getStats();
});

// REST: Send message
app.post<{ Body: { to: string; message: string; priority?: string }; Headers: { "x-agent-id"?: string } }>(
  "/api/messages",
  async (request, reply) => {
    const rawAgentId = request.headers["x-agent-id"];
    if (!rawAgentId) return reply.status(400).send({ error: "X-Agent-ID header required" });
    const agentId = normalizeAgentId(rawAgentId);
    const { to, message, priority = "normal" } = request.body;
    const normalizedTo = normalizeAgentId(to);
    const msg = await store.sendMessage(agentId, normalizedTo, message, priority);
    return { success: true, message_id: msg.id, from: agentId, to: normalizedTo, delivered_at: msg.timestamp, priority };
  }
);

// REST: Read messages
app.get<{ Querystring: { unread_only?: string; limit?: string; mark_as_read?: string }; Headers: { "x-agent-id"?: string } }>(
  "/api/messages",
  async (request, reply) => {
    const rawAgentId = request.headers["x-agent-id"];
    if (!rawAgentId) return reply.status(400).send({ error: "X-Agent-ID header required" });
    const agentId = normalizeAgentId(rawAgentId);
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
    const rawAgentId = request.headers["x-agent-id"];
    if (!rawAgentId) return reply.status(400).send({ error: "X-Agent-ID header required" });
    const agentId = normalizeAgentId(rawAgentId);
    const { message_ids } = request.body;
    const result = await store.deleteMessages(agentId, message_ids);
    return { ...result, deleted_count: result.deleted.length };
  }
);

// =============================================
// Reactive Injection Endpoints
// =============================================

/**
 * Wrap JEKT message with standard header/footer for clear identification.
 * Applied server-side so ALL sources get consistent formatting.
 */
function wrapJektMessage(params: {
  message: string;
  sourceAgent: string;
  targetAgent: string;
  priority: "normal" | "urgent";
  prNumber?: number;
}): string {
  const timestamp = new Date().toISOString().replace('T', ' ').substring(0, 19);
  const priorityTag = params.priority === 'urgent' ? ' [URGENT]' : '';

  // GitHub sources can't receive jekt - reply via PR comments
  const isGitHubSource = params.sourceAgent.toLowerCase().startsWith('github');
  const replyInstructions = isGitHubSource && params.prNumber
    ? `Reply: mcp__github__add_issue_comment or gh pr comment ${params.prNumber}`
    : `Reply: mcp__agentmux__inject_terminal to ${params.sourceAgent}`;

  return `JEKT | From: ${params.sourceAgent} | To: ${params.targetAgent}${priorityTag} | ${timestamp}
────────────────────────────────────────────────────────────
${params.message}
────────────────────────────────────────────────────────────
${replyInstructions}`;
}

// POST /reactive/inject - Create a new injection for cross-host delivery
app.post<{ Body: { target_agent: string; message: string; priority?: "normal" | "urgent"; pr_number?: number }; Headers: { "x-agent-id"?: string } }>(
  "/reactive/inject",
  async (request, reply) => {
    const rawSourceAgent = request.headers["x-agent-id"];
    if (!rawSourceAgent) return reply.status(400).send({ error: "X-Agent-ID header required" });
    // Don't normalize GitHub sources - preserve "GitHub (@user)" format
    const sourceAgent = rawSourceAgent.toLowerCase().startsWith('github')
      ? rawSourceAgent
      : normalizeAgentId(rawSourceAgent);

    const { target_agent, message, priority = "normal", pr_number } = request.body;
    if (!target_agent || !message) {
      return reply.status(400).send({ error: "target_agent and message are required" });
    }
    const normalizedTarget = normalizeAgentId(target_agent);

    // Validate message length (max 10KB)
    if (message.length > 10240) {
      return reply.status(400).send({ error: "message exceeds maximum length of 10KB" });
    }

    // Wrap message with standard JEKT format
    const wrappedMessage = wrapJektMessage({
      message,
      sourceAgent,
      targetAgent: normalizedTarget,
      priority: priority as "normal" | "urgent",
      prNumber: pr_number
    });

    const injection = await store.createInjection(sourceAgent, normalizedTarget, wrappedMessage, priority);
    return {
      success: true,
      injection_id: injection.id,
      source_agent: sourceAgent,
      target_agent: normalizedTarget,
      priority,
      created_at: injection.created_at,
      ttl_seconds: 3600
    };
  }
);

// GET /reactive/pending/:agent_id - Get pending injections for an agent
app.get<{ Params: { agent_id: string }; Headers: { "x-agent-id"?: string } }>(
  "/reactive/pending/:agent_id",
  async (request, reply) => {
    const rawCallerAgent = request.headers["x-agent-id"];
    if (!rawCallerAgent) return reply.status(400).send({ error: "X-Agent-ID header required" });
    const callerAgent = normalizeAgentId(rawCallerAgent);

    const { agent_id } = request.params;
    if (!agent_id) {
      return reply.status(400).send({ error: "agent_id parameter required" });
    }
    const normalizedAgentId = normalizeAgentId(agent_id);

    // Security: Verify caller is requesting their own injections (case-insensitive)
    if (callerAgent !== normalizedAgentId) {
      return reply.status(403).send({ error: "Not authorized - can only fetch own pending injections" });
    }

    const injections = await store.getPendingInjections(normalizedAgentId);
    return {
      agent_id: normalizedAgentId,
      injections: injections.map(inj => ({
        id: inj.id,
        source_agent: inj.source_agent,
        message: inj.message,
        priority: inj.priority,
        created_at: inj.created_at
      })),
      count: injections.length
    };
  }
);

// POST /reactive/ack - Acknowledge delivered injections
app.post<{ Body: { injection_ids: string[] }; Headers: { "x-agent-id"?: string } }>(
  "/reactive/ack",
  async (request, reply) => {
    const rawAgentId = request.headers["x-agent-id"];
    if (!rawAgentId) return reply.status(400).send({ error: "X-Agent-ID header required" });
    const agentId = normalizeAgentId(rawAgentId);

    const { injection_ids } = request.body;
    if (!injection_ids || !Array.isArray(injection_ids)) {
      return reply.status(400).send({ error: "injection_ids array required" });
    }

    // Pass agentId for authorization check (only target agent can acknowledge)
    const result = await store.acknowledgeInjections(agentId, injection_ids);
    return {
      agent_id: agentId,
      ...result,
      acknowledged_count: result.acknowledged.length
    };
  }
);

// GET /reactive/status/:injection_id - Check injection delivery status
app.get<{ Params: { injection_id: string }; Headers: { "x-agent-id"?: string } }>(
  "/reactive/status/:injection_id",
  async (request, reply) => {
    const rawCallerAgent = request.headers["x-agent-id"];
    if (!rawCallerAgent) return reply.status(400).send({ error: "X-Agent-ID header required" });
    const callerAgent = normalizeAgentId(rawCallerAgent);

    const { injection_id } = request.params;
    if (!injection_id) {
      return reply.status(400).send({ error: "injection_id parameter required" });
    }

    const injection = await store.getInjection(injection_id);
    if (!injection) {
      return reply.status(404).send({ error: "Injection not found or expired" });
    }

    // Security: Only source or target agent can check status
    // Don't normalize GitHub sources - preserve "GitHub (@user)" format
    const normalizedSource = injection.source_agent.toLowerCase().startsWith('github')
      ? injection.source_agent.toLowerCase()
      : normalizeAgentId(injection.source_agent);
    const normalizedTarget = normalizeAgentId(injection.target_agent);

    if (callerAgent !== normalizedSource && callerAgent !== normalizedTarget) {
      return reply.status(403).send({ error: "Not authorized to check this injection status" });
    }

    return {
      injection_id: injection.id,
      status: injection.status,
      target_agent: injection.target_agent,
      source_agent: injection.source_agent,
      created_at: injection.created_at,
      delivered_at: injection.delivered_at || null
    };
  }
);

// MCP Tools Definition
const MCP_TOOLS = [
  { name: "send_message", description: "Send a message to another agent", inputSchema: { type: "object", properties: { to: { type: "string" }, message: { type: "string" }, priority: { type: "string", enum: ["low","normal","high","urgent"], default: "normal" } }, required: ["to","message"] } },
  { name: "read_messages", description: "Read messages sent to this agent", inputSchema: { type: "object", properties: { unread_only: { type: "boolean", default: true }, limit: { type: "number", default: 10 }, mark_as_read: { type: "boolean", default: true } } } },
  { name: "list_agents", description: "List all agents", inputSchema: { type: "object", properties: {} } },
  { name: "broadcast_message", description: "Send to all agents", inputSchema: { type: "object", properties: { message: { type: "string" }, priority: { type: "string", default: "normal" } }, required: ["message"] } },
  { name: "delete_messages", description: "Delete messages by ID", inputSchema: { type: "object", properties: { message_ids: { type: "array", items: { type: "string" } } }, required: ["message_ids"] } }
];

interface MCPRequest { jsonrpc: "2.0"; id: number | string; method: string; params?: Record<string, unknown>; }

// MCP endpoint info (for clients checking availability)
app.get("/mcp", async (request, reply) => {
  return {
    name: "agentmux-server",
    version: "1.1.0",
    protocol: "JSON-RPC over HTTP",
    transport: "POST only (SSE not implemented)",
    usage: "Send JSON-RPC 2.0 requests via POST with X-Agent-ID header"
  };
});

// MCP JSON-RPC endpoint
app.post<{ Body: MCPRequest; Headers: { "x-agent-id"?: string } }>("/mcp", async (request, reply) => {
  const rawAgentId = request.headers["x-agent-id"];
  if (!rawAgentId) {
    return reply.status(400).send({ jsonrpc: "2.0", id: null, error: { code: -32600, message: "X-Agent-ID header required" } });
  }
  const agentId = normalizeAgentId(rawAgentId);
  const { jsonrpc, id, method, params } = request.body;
  if (jsonrpc !== "2.0") return reply.status(400).send({ jsonrpc: "2.0", id, error: { code: -32600, message: "Invalid JSON-RPC" } });

  try {
    let result: unknown;
    switch (method) {
      case "initialize":
        result = { protocolVersion: "2024-11-05", capabilities: { tools: {} }, serverInfo: { name: "agentmux-server", version: "1.1.0" } };
        break;
      case "tools/list":
        result = { tools: MCP_TOOLS };
        break;
      case "tools/call": {
        const toolName = (params as any)?.name;
        const args = (params as any)?.arguments || {};
        switch (toolName) {
          case "send_message": {
            const normalizedTo = normalizeAgentId(args.to);
            const msg = await store.sendMessage(agentId, normalizedTo, args.message, args.priority || "normal");
            result = { content: [{ type: "text", text: JSON.stringify({ success: true, message_id: msg.id, from: agentId, to: normalizedTo, delivered_at: msg.timestamp, priority: args.priority || "normal" }, null, 2) }] };
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
    console.log(`AgentMux Server v1.1.0 listening on http://${HOST}:${PORT}`);
    console.log(`MCP endpoint: POST http://${HOST}:${PORT}/mcp`);
    console.log(`REST API: http://${HOST}:${PORT}/api/*`);
  } catch (err) {
    app.log.error(err);
    process.exit(1);
  }
}

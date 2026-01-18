/**
 * App builder - separated for testability
 */
import Fastify, { FastifyInstance } from "fastify";
import cors from "@fastify/cors";
import { IMessageStore, Message, Agent, Injection } from "./store.js";

// MCP Tools Definition
export const MCP_TOOLS = [
  { name: "send_message", description: "Send a message to another agent", inputSchema: { type: "object", properties: { to: { type: "string" }, message: { type: "string" }, priority: { type: "string", enum: ["low","normal","high","urgent"], default: "normal" } }, required: ["to","message"] } },
  { name: "read_messages", description: "Read messages sent to this agent", inputSchema: { type: "object", properties: { unread_only: { type: "boolean", default: true }, limit: { type: "number", default: 10 }, mark_as_read: { type: "boolean", default: true } } } },
  { name: "list_agents", description: "List all agents", inputSchema: { type: "object", properties: {} } },
  { name: "broadcast_message", description: "Send to all agents", inputSchema: { type: "object", properties: { message: { type: "string" }, priority: { type: "string", default: "normal" } }, required: ["message"] } },
  { name: "delete_messages", description: "Delete messages by ID", inputSchema: { type: "object", properties: { message_ids: { type: "array", items: { type: "string" } } }, required: ["message_ids"] } }
];

interface MCPRequest { jsonrpc: "2.0"; id: number | string; method: string; params?: Record<string, unknown>; }

export interface AppOptions {
  store: IMessageStore;
  getApiKey: () => Promise<string>;
  logger?: boolean;
}

export async function buildApp(options: AppOptions): Promise<FastifyInstance> {
  const { store, getApiKey, logger = false } = options;
  const app = Fastify({ logger });

  await app.register(cors, { origin: true });

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
      const sourceAgent = request.headers["x-agent-id"];
      if (!sourceAgent) return reply.status(400).send({ error: "X-Agent-ID header required" });

      const { target_agent, message, priority = "normal", pr_number } = request.body;
      if (!target_agent || !message) {
        return reply.status(400).send({ error: "target_agent and message are required" });
      }

      // Validate message length (max 10KB)
      if (message.length > 10240) {
        return reply.status(400).send({ error: "message exceeds maximum length of 10KB" });
      }

      // Wrap message with standard JEKT format
      const wrappedMessage = wrapJektMessage({
        message,
        sourceAgent,
        targetAgent: target_agent,
        priority: priority as "normal" | "urgent",
        prNumber: pr_number
      });

      const injection = await store.createInjection(sourceAgent, target_agent, wrappedMessage, priority);
      return {
        success: true,
        injection_id: injection.id,
        source_agent: sourceAgent,
        target_agent,
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
      const callerAgent = request.headers["x-agent-id"];
      if (!callerAgent) return reply.status(400).send({ error: "X-Agent-ID header required" });

      const { agent_id } = request.params;
      if (!agent_id) {
        return reply.status(400).send({ error: "agent_id parameter required" });
      }

      // Security: Verify caller is requesting their own injections
      if (callerAgent !== agent_id) {
        return reply.status(403).send({ error: "Not authorized - can only fetch own pending injections" });
      }

      const injections = await store.getPendingInjections(agent_id);
      return {
        agent_id,
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
      const agentId = request.headers["x-agent-id"];
      if (!agentId) return reply.status(400).send({ error: "X-Agent-ID header required" });

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

  // MCP endpoint info (for clients checking availability)
  app.get("/mcp", async () => {
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

  return app;
}

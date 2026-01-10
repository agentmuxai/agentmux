import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { buildApp, MCP_TOOLS } from './app.js';
import type { Message, Agent, IMessageStore } from './store.js';

// Mock store implementation for testing
function createMockStore(): IMessageStore & { messages: Message[]; agents: Agent[] } {
  const messages: Message[] = [];
  const agents: Agent[] = [];

  return {
    messages,
    agents,

    async sendMessage(from: string, to: string, text: string, priority: string = 'normal'): Promise<Message> {
      const msg: Message = {
        id: `msg-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`,
        from_agent: from,
        to_agent: to,
        text,
        priority: priority as Message['priority'],
        timestamp: new Date().toISOString(),
        read: false,
      };
      messages.push(msg);

      // Update agent
      const existingAgent = agents.find(a => a.id === from);
      if (existingAgent) {
        existingAgent.last_seen = msg.timestamp;
        existingAgent.messages_sent++;
      } else {
        agents.push({ id: from, last_seen: msg.timestamp, messages_sent: 1 });
      }

      return msg;
    },

    async readMessages(agentId: string, unreadOnly: boolean = true, limit: number = 10, markAsRead: boolean = true): Promise<Message[]> {
      let filtered = messages.filter(m => m.to_agent === agentId || m.to_agent === '*');
      if (unreadOnly) {
        filtered = filtered.filter(m => !m.read);
      }
      const result = filtered.slice(0, limit);
      if (markAsRead) {
        result.forEach(m => m.read = true);
      }
      return result;
    },

    async listAgents(): Promise<Agent[]> {
      return [...agents];
    },

    async deleteMessages(agentId: string, messageIds: string[]): Promise<{ deleted: string[]; errors: { id: string; error: string }[] }> {
      const deleted: string[] = [];
      const errors: { id: string; error: string }[] = [];

      for (const id of messageIds) {
        const idx = messages.findIndex(m => m.id === id);
        if (idx === -1) {
          errors.push({ id, error: 'Message not found' });
        } else {
          const msg = messages[idx];
          if (msg.to_agent !== agentId && msg.from_agent !== agentId && msg.to_agent !== '*') {
            errors.push({ id, error: 'Not authorized' });
          } else {
            messages.splice(idx, 1);
            deleted.push(id);
          }
        }
      }

      return { deleted, errors };
    },

    async getStats() {
      return {
        total_messages: messages.length,
        unread_messages: messages.filter(m => !m.read).length,
        unique_agents: agents.length,
      };
    },
  };
}

const TEST_API_KEY = 'test-api-key-12345';

describe('AgentMux Server', () => {
  describe('Health endpoint', () => {
    it('returns health status without auth', async () => {
      const store = createMockStore();
      const app = await buildApp({ store, getApiKey: async () => TEST_API_KEY });

      const response = await app.inject({
        method: 'GET',
        url: '/api/health',
      });

      expect(response.statusCode).toBe(200);
      const body = JSON.parse(response.body);
      expect(body.status).toBe('ok');
      expect(body.version).toBe('1.0.0');
      expect(body.timestamp).toBeDefined();
    });
  });

  describe('Authentication', () => {
    it('rejects requests without auth token', async () => {
      const store = createMockStore();
      const app = await buildApp({ store, getApiKey: async () => TEST_API_KEY });

      const response = await app.inject({
        method: 'GET',
        url: '/api/stats',
      });

      expect(response.statusCode).toBe(401);
      const body = JSON.parse(response.body);
      expect(body.error).toBe('Unauthorized');
    });

    it('rejects requests with invalid token', async () => {
      const store = createMockStore();
      const app = await buildApp({ store, getApiKey: async () => TEST_API_KEY });

      const response = await app.inject({
        method: 'GET',
        url: '/api/stats',
        headers: { authorization: 'Bearer wrong-token' },
      });

      expect(response.statusCode).toBe(401);
    });

    it('accepts requests with valid token', async () => {
      const store = createMockStore();
      const app = await buildApp({ store, getApiKey: async () => TEST_API_KEY });

      const response = await app.inject({
        method: 'GET',
        url: '/api/stats',
        headers: { authorization: `Bearer ${TEST_API_KEY}` },
      });

      expect(response.statusCode).toBe(200);
    });
  });

  describe('REST API - Messages', () => {
    it('requires X-Agent-ID header for sending messages', async () => {
      const store = createMockStore();
      const app = await buildApp({ store, getApiKey: async () => TEST_API_KEY });

      const response = await app.inject({
        method: 'POST',
        url: '/api/messages',
        headers: { authorization: `Bearer ${TEST_API_KEY}` },
        payload: { to: 'agent2', message: 'Hello' },
      });

      expect(response.statusCode).toBe(400);
      const body = JSON.parse(response.body);
      expect(body.error).toBe('X-Agent-ID header required');
    });

    it('sends a message successfully', async () => {
      const store = createMockStore();
      const app = await buildApp({ store, getApiKey: async () => TEST_API_KEY });

      const response = await app.inject({
        method: 'POST',
        url: '/api/messages',
        headers: {
          authorization: `Bearer ${TEST_API_KEY}`,
          'x-agent-id': 'agent1',
        },
        payload: { to: 'agent2', message: 'Hello there!' },
      });

      expect(response.statusCode).toBe(200);
      const body = JSON.parse(response.body);
      expect(body.success).toBe(true);
      expect(body.from).toBe('agent1');
      expect(body.to).toBe('agent2');
      expect(body.message_id).toBeDefined();
    });

    it('reads messages for an agent', async () => {
      const store = createMockStore();
      const app = await buildApp({ store, getApiKey: async () => TEST_API_KEY });

      // Send a message first
      await store.sendMessage('agent1', 'agent2', 'Test message', 'normal');

      const response = await app.inject({
        method: 'GET',
        url: '/api/messages',
        headers: {
          authorization: `Bearer ${TEST_API_KEY}`,
          'x-agent-id': 'agent2',
        },
      });

      expect(response.statusCode).toBe(200);
      const body = JSON.parse(response.body);
      expect(body.agent_id).toBe('agent2');
      expect(body.messages).toHaveLength(1);
      expect(body.messages[0].from).toBe('agent1');
      expect(body.messages[0].message).toBe('Test message');
    });
  });

  describe('REST API - Agents', () => {
    it('lists agents', async () => {
      const store = createMockStore();
      const app = await buildApp({ store, getApiKey: async () => TEST_API_KEY });

      // Create some agents by sending messages
      await store.sendMessage('agent1', 'agent2', 'Hello', 'normal');
      await store.sendMessage('agent3', 'agent1', 'Hi', 'normal');

      const response = await app.inject({
        method: 'GET',
        url: '/api/agents',
        headers: { authorization: `Bearer ${TEST_API_KEY}` },
      });

      expect(response.statusCode).toBe(200);
      const body = JSON.parse(response.body);
      expect(body.agents).toHaveLength(2);
      expect(body.total_count).toBe(2);
    });
  });

  describe('MCP Protocol', () => {
    it('returns server info on GET /mcp', async () => {
      const store = createMockStore();
      const app = await buildApp({ store, getApiKey: async () => TEST_API_KEY });

      const response = await app.inject({
        method: 'GET',
        url: '/mcp',
        headers: { authorization: `Bearer ${TEST_API_KEY}` },
      });

      expect(response.statusCode).toBe(200);
      const body = JSON.parse(response.body);
      expect(body.name).toBe('agentmux-server');
      expect(body.protocol).toBe('JSON-RPC over HTTP');
    });

    it('handles initialize method', async () => {
      const store = createMockStore();
      const app = await buildApp({ store, getApiKey: async () => TEST_API_KEY });

      const response = await app.inject({
        method: 'POST',
        url: '/mcp',
        headers: {
          authorization: `Bearer ${TEST_API_KEY}`,
          'x-agent-id': 'test-agent',
        },
        payload: { jsonrpc: '2.0', id: 1, method: 'initialize' },
      });

      expect(response.statusCode).toBe(200);
      const body = JSON.parse(response.body);
      expect(body.jsonrpc).toBe('2.0');
      expect(body.id).toBe(1);
      expect(body.result.protocolVersion).toBe('2024-11-05');
      expect(body.result.serverInfo.name).toBe('agentmux-server');
    });

    it('handles tools/list method', async () => {
      const store = createMockStore();
      const app = await buildApp({ store, getApiKey: async () => TEST_API_KEY });

      const response = await app.inject({
        method: 'POST',
        url: '/mcp',
        headers: {
          authorization: `Bearer ${TEST_API_KEY}`,
          'x-agent-id': 'test-agent',
        },
        payload: { jsonrpc: '2.0', id: 2, method: 'tools/list' },
      });

      expect(response.statusCode).toBe(200);
      const body = JSON.parse(response.body);
      expect(body.result.tools).toHaveLength(5);
      expect(body.result.tools.map((t: any) => t.name)).toContain('send_message');
      expect(body.result.tools.map((t: any) => t.name)).toContain('read_messages');
    });

    it('handles tools/call send_message', async () => {
      const store = createMockStore();
      const app = await buildApp({ store, getApiKey: async () => TEST_API_KEY });

      const response = await app.inject({
        method: 'POST',
        url: '/mcp',
        headers: {
          authorization: `Bearer ${TEST_API_KEY}`,
          'x-agent-id': 'sender-agent',
        },
        payload: {
          jsonrpc: '2.0',
          id: 3,
          method: 'tools/call',
          params: {
            name: 'send_message',
            arguments: { to: 'receiver-agent', message: 'Hello via MCP!' },
          },
        },
      });

      expect(response.statusCode).toBe(200);
      const body = JSON.parse(response.body);
      expect(body.result.content).toHaveLength(1);

      const content = JSON.parse(body.result.content[0].text);
      expect(content.success).toBe(true);
      expect(content.from).toBe('sender-agent');
      expect(content.to).toBe('receiver-agent');
    });

    it('handles tools/call read_messages', async () => {
      const store = createMockStore();
      const app = await buildApp({ store, getApiKey: async () => TEST_API_KEY });

      // Pre-populate a message
      await store.sendMessage('agent1', 'reader-agent', 'Unread message', 'high');

      const response = await app.inject({
        method: 'POST',
        url: '/mcp',
        headers: {
          authorization: `Bearer ${TEST_API_KEY}`,
          'x-agent-id': 'reader-agent',
        },
        payload: {
          jsonrpc: '2.0',
          id: 4,
          method: 'tools/call',
          params: { name: 'read_messages', arguments: {} },
        },
      });

      expect(response.statusCode).toBe(200);
      const body = JSON.parse(response.body);
      const content = JSON.parse(body.result.content[0].text);
      expect(content.agent_id).toBe('reader-agent');
      expect(content.messages).toHaveLength(1);
      expect(content.messages[0].priority).toBe('high');
    });

    it('rejects invalid JSON-RPC version', async () => {
      const store = createMockStore();
      const app = await buildApp({ store, getApiKey: async () => TEST_API_KEY });

      const response = await app.inject({
        method: 'POST',
        url: '/mcp',
        headers: {
          authorization: `Bearer ${TEST_API_KEY}`,
          'x-agent-id': 'test-agent',
        },
        payload: { jsonrpc: '1.0', id: 1, method: 'initialize' },
      });

      expect(response.statusCode).toBe(400);
      const body = JSON.parse(response.body);
      expect(body.error.code).toBe(-32600);
    });

    it('returns error for unknown method', async () => {
      const store = createMockStore();
      const app = await buildApp({ store, getApiKey: async () => TEST_API_KEY });

      const response = await app.inject({
        method: 'POST',
        url: '/mcp',
        headers: {
          authorization: `Bearer ${TEST_API_KEY}`,
          'x-agent-id': 'test-agent',
        },
        payload: { jsonrpc: '2.0', id: 1, method: 'unknown/method' },
      });

      expect(response.statusCode).toBe(400);
      const body = JSON.parse(response.body);
      expect(body.error.code).toBe(-32601);
    });
  });

  describe('MCP_TOOLS definition', () => {
    it('has all required tools', () => {
      const toolNames = MCP_TOOLS.map(t => t.name);
      expect(toolNames).toContain('send_message');
      expect(toolNames).toContain('read_messages');
      expect(toolNames).toContain('list_agents');
      expect(toolNames).toContain('broadcast_message');
      expect(toolNames).toContain('delete_messages');
    });

    it('has valid input schemas', () => {
      for (const tool of MCP_TOOLS) {
        expect(tool.inputSchema).toBeDefined();
        expect(tool.inputSchema.type).toBe('object');
        expect(tool.inputSchema.properties).toBeDefined();
      }
    });
  });
});

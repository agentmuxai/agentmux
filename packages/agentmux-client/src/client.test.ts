import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { jsonRpcCall, getConfigFromEnv, TOOLS, type AgentMuxConfig } from './client.js';

describe('client', () => {
  describe('getConfigFromEnv', () => {
    const originalEnv = process.env;

    beforeEach(() => {
      vi.resetModules();
      process.env = { ...originalEnv };
    });

    afterEach(() => {
      process.env = originalEnv;
    });

    it('returns default values when no env vars set', () => {
      delete process.env.AGENTMUX_URL;
      delete process.env.AGENTMUX_AGENT_ID;
      delete process.env.AGENT_NAME;
      delete process.env.AGENTMUX_TOKEN;

      const config = getConfigFromEnv();

      expect(config.url).toBe('https://agentmux.asaf.cc');
      expect(config.agentId).toBe('unknown-agent');
      expect(config.token).toBeUndefined();
    });

    it('reads AGENTMUX_URL from environment', () => {
      process.env.AGENTMUX_URL = 'https://custom.example.com';

      const config = getConfigFromEnv();

      expect(config.url).toBe('https://custom.example.com');
    });

    it('reads AGENTMUX_AGENT_ID from environment', () => {
      process.env.AGENTMUX_AGENT_ID = 'test-agent';

      const config = getConfigFromEnv();

      expect(config.agentId).toBe('test-agent');
    });

    it('falls back to AGENT_NAME when AGENTMUX_AGENT_ID not set', () => {
      delete process.env.AGENTMUX_AGENT_ID;
      process.env.AGENT_NAME = 'fallback-agent';

      const config = getConfigFromEnv();

      expect(config.agentId).toBe('fallback-agent');
    });

    it('prefers AGENTMUX_AGENT_ID over AGENT_NAME', () => {
      process.env.AGENTMUX_AGENT_ID = 'primary-agent';
      process.env.AGENT_NAME = 'fallback-agent';

      const config = getConfigFromEnv();

      expect(config.agentId).toBe('primary-agent');
    });

    it('reads AGENTMUX_TOKEN from environment', () => {
      process.env.AGENTMUX_TOKEN = 'secret-token';

      const config = getConfigFromEnv();

      expect(config.token).toBe('secret-token');
    });
  });

  describe('jsonRpcCall', () => {
    const mockConfig: AgentMuxConfig = {
      url: 'https://test.example.com',
      agentId: 'test-agent',
      token: 'test-token',
    };

    beforeEach(() => {
      vi.stubGlobal('fetch', vi.fn());
    });

    afterEach(() => {
      vi.unstubAllGlobals();
    });

    it('sends correct headers with token', async () => {
      const mockFetch = vi.fn().mockResolvedValue({
        ok: true,
        json: () => Promise.resolve({ jsonrpc: '2.0', id: 1, result: { success: true } }),
      });
      vi.stubGlobal('fetch', mockFetch);

      await jsonRpcCall(mockConfig, 'test_method', { foo: 'bar' });

      expect(mockFetch).toHaveBeenCalledWith(
        'https://test.example.com/mcp',
        expect.objectContaining({
          method: 'POST',
          headers: expect.objectContaining({
            'Content-Type': 'application/json',
            'X-Agent-ID': 'test-agent',
            'Authorization': 'Bearer test-token',
          }),
        })
      );
    });

    it('omits Authorization header when no token', async () => {
      const configWithoutToken: AgentMuxConfig = {
        url: 'https://test.example.com',
        agentId: 'test-agent',
      };

      const mockFetch = vi.fn().mockResolvedValue({
        ok: true,
        json: () => Promise.resolve({ jsonrpc: '2.0', id: 1, result: {} }),
      });
      vi.stubGlobal('fetch', mockFetch);

      await jsonRpcCall(configWithoutToken, 'test_method');

      const [, options] = mockFetch.mock.calls[0];
      expect(options.headers).not.toHaveProperty('Authorization');
    });

    it('sends correct JSON-RPC body', async () => {
      const mockFetch = vi.fn().mockResolvedValue({
        ok: true,
        json: () => Promise.resolve({ jsonrpc: '2.0', id: 1, result: {} }),
      });
      vi.stubGlobal('fetch', mockFetch);

      await jsonRpcCall(mockConfig, 'tools/call', { name: 'send_message', arguments: { to: 'agent1' } });

      const [, options] = mockFetch.mock.calls[0];
      const body = JSON.parse(options.body);

      expect(body.jsonrpc).toBe('2.0');
      expect(body.method).toBe('tools/call');
      expect(body.params).toEqual({ name: 'send_message', arguments: { to: 'agent1' } });
      expect(body.id).toBeTypeOf('number');
    });

    it('returns result on success', async () => {
      const mockFetch = vi.fn().mockResolvedValue({
        ok: true,
        json: () => Promise.resolve({
          jsonrpc: '2.0',
          id: 1,
          result: { messages: [{ id: '1', content: 'Hello' }] },
        }),
      });
      vi.stubGlobal('fetch', mockFetch);

      const result = await jsonRpcCall(mockConfig, 'read_messages');

      expect(result).toEqual({ messages: [{ id: '1', content: 'Hello' }] });
    });

    it('throws on HTTP error', async () => {
      const mockFetch = vi.fn().mockResolvedValue({
        ok: false,
        status: 500,
        statusText: 'Internal Server Error',
      });
      vi.stubGlobal('fetch', mockFetch);

      await expect(jsonRpcCall(mockConfig, 'test_method')).rejects.toThrow(
        'HTTP 500: Internal Server Error'
      );
    });

    it('throws on JSON-RPC error', async () => {
      const mockFetch = vi.fn().mockResolvedValue({
        ok: true,
        json: () => Promise.resolve({
          jsonrpc: '2.0',
          id: 1,
          error: { code: -32600, message: 'Invalid Request' },
        }),
      });
      vi.stubGlobal('fetch', mockFetch);

      await expect(jsonRpcCall(mockConfig, 'test_method')).rejects.toThrow(
        'JSON-RPC Error: Invalid Request'
      );
    });
  });

  describe('TOOLS', () => {
    it('defines send_message tool', () => {
      const tool = TOOLS.find(t => t.name === 'send_message');
      expect(tool).toBeDefined();
      expect(tool?.inputSchema.required).toContain('to');
      expect(tool?.inputSchema.required).toContain('message');
    });

    it('defines read_messages tool', () => {
      const tool = TOOLS.find(t => t.name === 'read_messages');
      expect(tool).toBeDefined();
      expect(tool?.inputSchema.properties).toHaveProperty('unread_only');
      expect(tool?.inputSchema.properties).toHaveProperty('limit');
    });

    it('defines list_agents tool', () => {
      const tool = TOOLS.find(t => t.name === 'list_agents');
      expect(tool).toBeDefined();
    });

    it('defines broadcast_message tool', () => {
      const tool = TOOLS.find(t => t.name === 'broadcast_message');
      expect(tool).toBeDefined();
      expect(tool?.inputSchema.required).toContain('message');
    });

    it('defines delete_messages tool', () => {
      const tool = TOOLS.find(t => t.name === 'delete_messages');
      expect(tool).toBeDefined();
      expect(tool?.inputSchema.required).toContain('message_ids');
    });

    it('defines inject_terminal tool', () => {
      const tool = TOOLS.find(t => t.name === 'inject_terminal');
      expect(tool).toBeDefined();
      expect(tool?.inputSchema.required).toContain('target_agent');
      expect(tool?.inputSchema.required).toContain('message');
      expect(tool?.inputSchema.properties).toHaveProperty('priority');
    });

    it('has 6 tools total', () => {
      expect(TOOLS).toHaveLength(6);
    });
  });
});

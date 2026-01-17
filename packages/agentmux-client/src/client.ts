/**
 * AgentMux Client - Core functionality
 * Separated from index.ts for testability
 */

export interface AgentMuxConfig {
  url: string;
  agentId: string;
  token?: string;
}

export interface JsonRpcResponse {
  jsonrpc: string;
  id: number;
  result?: any;
  error?: { code: number; message: string };
}

/**
 * Make JSON-RPC call to AgentMux HTTP endpoint
 */
export async function jsonRpcCall(
  config: AgentMuxConfig,
  method: string,
  params: any = {}
): Promise<any> {
  const headers: Record<string, string> = {
    'Content-Type': 'application/json',
    'X-Agent-ID': config.agentId,
  };

  // Add auth header if token is available
  if (config.token) {
    headers['Authorization'] = `Bearer ${config.token}`;
  }

  const response = await fetch(`${config.url}/mcp`, {
    method: 'POST',
    headers,
    body: JSON.stringify({
      jsonrpc: '2.0',
      id: Date.now(),
      method,
      params,
    }),
  });

  if (!response.ok) {
    throw new Error(`HTTP ${response.status}: ${response.statusText}`);
  }

  const data = (await response.json()) as JsonRpcResponse;

  if (data.error) {
    throw new Error(`JSON-RPC Error: ${data.error.message}`);
  }

  return data.result;
}

/**
 * Get configuration from environment variables
 */
export function getConfigFromEnv(): AgentMuxConfig {
  return {
    url: process.env.AGENTMUX_URL || 'https://agentmux.asaf.cc',
    agentId: process.env.AGENTMUX_AGENT_ID || process.env.AGENT_NAME || 'unknown-agent',
    token: process.env.AGENTMUX_TOKEN,
  };
}

/**
 * Tool definitions for MCP
 */
export const TOOLS = [
  {
    name: 'send_message',
    description: 'Send a message to another agent',
    inputSchema: {
      type: 'object',
      properties: {
        to: {
          type: 'string',
          description: 'Agent ID to send message to',
        },
        message: {
          type: 'string',
          description: 'Message content',
        },
        priority: {
          type: 'string',
          enum: ['low', 'normal', 'high', 'urgent'],
          description: 'Message priority',
          default: 'normal',
        },
      },
      required: ['to', 'message'],
    },
  },
  {
    name: 'read_messages',
    description: 'Read messages for this agent',
    inputSchema: {
      type: 'object',
      properties: {
        unread_only: {
          type: 'boolean',
          description: 'Only return unread messages',
          default: true,
        },
        limit: {
          type: 'number',
          description: 'Maximum number of messages to return',
          default: 100,
        },
      },
    },
  },
  {
    name: 'list_agents',
    description: 'List all known agents',
    inputSchema: {
      type: 'object',
      properties: {},
    },
  },
  {
    name: 'broadcast_message',
    description: 'Send a message to all agents',
    inputSchema: {
      type: 'object',
      properties: {
        message: {
          type: 'string',
          description: 'Message content',
        },
        priority: {
          type: 'string',
          enum: ['low', 'normal', 'high', 'urgent'],
          description: 'Message priority',
          default: 'normal',
        },
      },
      required: ['message'],
    },
  },
  {
    name: 'delete_messages',
    description: 'Delete messages by ID',
    inputSchema: {
      type: 'object',
      properties: {
        message_ids: {
          type: 'array',
          items: { type: 'string' },
          description: 'Array of message IDs to delete',
        },
      },
      required: ['message_ids'],
    },
  },
  {
    name: 'inject_terminal',
    description: 'Inject a message directly into another agent\'s terminal (reactive messaging). The message appears as user input and is processed immediately by the target agent.',
    inputSchema: {
      type: 'object',
      properties: {
        target_agent: {
          type: 'string',
          description: 'Agent ID to inject message into (e.g., "AgentX", "AgentA")',
        },
        message: {
          type: 'string',
          description: 'The message to inject as user input',
        },
        priority: {
          type: 'string',
          enum: ['normal', 'urgent'],
          description: 'Message priority (urgent may interrupt current processing)',
          default: 'normal',
        },
      },
      required: ['target_agent', 'message'],
    },
  },
];

/**
 * Inject a message into a target agent's terminal via AgentMux cloud.
 *
 * Always routes through AgentMux for reliable cross-host delivery.
 * The target WaveMux instance polls for pending injections and delivers locally.
 */
export async function injectTerminal(
  targetAgent: string,
  message: string,
  sourceAgent: string,
  priority: string = 'normal'
): Promise<any> {
  const config = getConfigFromEnv();

  if (!config.token) {
    throw new Error('inject_terminal requires AGENTMUX_TOKEN to be set');
  }

  const response = await fetch(`${config.url}/reactive/inject`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      'Authorization': `Bearer ${config.token}`,
      'X-Agent-ID': sourceAgent,
    },
    body: JSON.stringify({
      target_agent: targetAgent,
      message: message,
      source_agent: sourceAgent,
      priority: priority,
    }),
  });

  if (!response.ok) {
    let errorMsg = `HTTP ${response.status}`;
    try {
      const data = await response.json() as { error?: string };
      errorMsg = data.error || errorMsg;
    } catch {
      // Response not JSON
    }
    throw new Error(`Injection failed: ${errorMsg}`);
  }

  return await response.json();
}

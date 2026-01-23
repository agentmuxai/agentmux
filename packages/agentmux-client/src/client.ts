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
 * Check injection delivery status from AgentMux.
 */
async function checkInjectionStatus(
  injectionId: string,
  sourceAgent: string,
  config: AgentMuxConfig
): Promise<{ status: string; delivered_at?: string }> {
  const response = await fetch(`${config.url}/reactive/status/${injectionId}`, {
    method: 'GET',
    headers: {
      'Authorization': `Bearer ${config.token}`,
      'X-Agent-ID': sourceAgent,
    },
  });

  if (!response.ok) {
    if (response.status === 404) {
      return { status: 'expired' };
    }
    throw new Error(`Status check failed: HTTP ${response.status}`);
  }

  return await response.json() as { status: string; delivered_at?: string };
}

/**
 * Sleep helper for polling
 */
function sleep(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms));
}

/** Result type for inject_terminal with delivery confirmation */
export interface InjectTerminalResult {
  success: boolean;
  injection_id: string;
  status: 'delivered' | 'pending' | 'timeout';
  error?: string;
  target_agent: string;
  delivered_at?: string;
}

/**
 * Inject a message into a target agent's terminal via AgentMux cloud.
 *
 * Always routes through AgentMux for reliable cross-host delivery.
 * The target WaveMux instance polls for pending injections and delivers locally.
 *
 * Now includes delivery confirmation - waits for WaveMux to acknowledge delivery
 * before returning success. This prevents false positives when target agent is
 * offline or unregistered.
 *
 * @param targetAgent - Agent ID to inject message into
 * @param message - Message content
 * @param sourceAgent - Source agent ID (for logging/attribution)
 * @param priority - Message priority (normal/urgent)
 * @param timeoutMs - Max time to wait for delivery confirmation (default: 15000ms)
 */
export async function injectTerminal(
  targetAgent: string,
  message: string,
  sourceAgent: string,
  priority: string = 'normal',
  timeoutMs: number = 15000
): Promise<InjectTerminalResult> {
  const config = getConfigFromEnv();

  if (!config.token) {
    throw new Error('inject_terminal requires AGENTMUX_TOKEN to be set');
  }

  // Step 1: Create the injection
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
    return {
      success: false,
      injection_id: '',
      status: 'pending',
      error: `Injection failed: ${errorMsg}`,
      target_agent: targetAgent,
    };
  }

  const injectResult = await response.json() as { injection_id: string; success: boolean };

  if (!injectResult.success || !injectResult.injection_id) {
    return {
      success: false,
      injection_id: injectResult.injection_id || '',
      status: 'pending',
      error: 'Failed to create injection',
      target_agent: targetAgent,
    };
  }

  // Step 2: Poll for delivery confirmation
  const pollInterval = 1000; // 1 second
  const startTime = Date.now();

  while (Date.now() - startTime < timeoutMs) {
    await sleep(pollInterval);

    try {
      const statusResult = await checkInjectionStatus(
        injectResult.injection_id,
        sourceAgent,
        config
      );

      if (statusResult.status === 'delivered') {
        return {
          success: true,
          injection_id: injectResult.injection_id,
          status: 'delivered',
          target_agent: targetAgent,
          delivered_at: statusResult.delivered_at,
        };
      }

      if (statusResult.status === 'expired') {
        return {
          success: false,
          injection_id: injectResult.injection_id,
          status: 'timeout',
          error: 'Injection expired before delivery',
          target_agent: targetAgent,
        };
      }
    } catch (err) {
      // Continue polling on transient errors
      console.error(`[AgentMux] Status check error: ${err}`);
    }
  }

  // Timeout - delivery not confirmed
  return {
    success: false,
    injection_id: injectResult.injection_id,
    status: 'timeout',
    error: `Delivery not confirmed within ${timeoutMs / 1000}s. Target agent may be offline or unregistered.`,
    target_agent: targetAgent,
  };
}

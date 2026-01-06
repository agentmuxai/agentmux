#!/usr/bin/env node

import { Server } from '@modelcontextprotocol/sdk/server/index.js';
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js';
import {
  CallToolRequestSchema,
  ListToolsRequestSchema,
} from '@modelcontextprotocol/sdk/types.js';

// Configuration from environment
const AGENTMUX_URL = process.env.AGENTMUX_URL ||
  'https://xv7wycacd3vmglr7j24cfdkhb40buykg.lambda-url.us-east-1.on.aws';
const AGENT_ID = process.env.AGENTMUX_AGENT_ID || process.env.AGENT_NAME || 'unknown-agent';

/**
 * Make JSON-RPC call to AgentMux HTTP endpoint
 */
async function jsonRpcCall(method: string, params: any = {}): Promise<any> {
  const response = await fetch(`${AGENTMUX_URL}/mcp`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      'X-Agent-ID': AGENT_ID,
    },
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

  const data = await response.json() as any;

  if (data.error) {
    throw new Error(`JSON-RPC Error: ${data.error.message}`);
  }

  return data.result;
}

// Create MCP server
const server = new Server(
  {
    name: 'agentmux',
    version: '1.0.0',
  },
  {
    capabilities: {
      tools: {},
    },
  }
);

// List available tools
server.setRequestHandler(ListToolsRequestSchema, async () => {
  return {
    tools: [
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
    ],
  };
});

// Handle tool calls
server.setRequestHandler(CallToolRequestSchema, async (request) => {
  const { name, arguments: args } = request.params;

  try {
    const result = await jsonRpcCall('tools/call', {
      name,
      arguments: args || {},
    });

    return {
      content: result.content || [
        {
          type: 'text',
          text: JSON.stringify(result, null, 2),
        },
      ],
    };
  } catch (error) {
    return {
      content: [
        {
          type: 'text',
          text: `Error: ${error instanceof Error ? error.message : String(error)}`,
        },
      ],
      isError: true,
    };
  }
});

// Start server
async function main() {
  // Start MCP server
  const transport = new StdioServerTransport();
  await server.connect(transport);

  console.error('[AgentMux MCP] Stdio wrapper started');
  console.error(`[AgentMux MCP] Agent ID: ${AGENT_ID}`);
  console.error(`[AgentMux MCP] Lambda URL: ${AGENTMUX_URL}`);
}

main().catch((error) => {
  console.error('Fatal error:', error);
  process.exit(1);
});

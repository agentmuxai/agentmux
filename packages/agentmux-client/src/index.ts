#!/usr/bin/env node

import { Server } from '@modelcontextprotocol/sdk/server/index.js';
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js';
import {
  CallToolRequestSchema,
  ListToolsRequestSchema,
} from '@modelcontextprotocol/sdk/types.js';
import { jsonRpcCall, getConfigFromEnv, TOOLS, injectTerminal } from './client.js';

// Get configuration
const config = getConfigFromEnv();

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
  return { tools: TOOLS };
});

// Handle tool calls
server.setRequestHandler(CallToolRequestSchema, async (request) => {
  const { name, arguments: args } = request.params;

  try {
    let result: any;

    // Handle inject_terminal locally (calls WaveMux directly)
    if (name === 'inject_terminal') {
      const typedArgs = args as { target_agent: string; message: string; priority?: string };
      result = await injectTerminal(
        typedArgs.target_agent,
        typedArgs.message,
        config.agentId,
        typedArgs.priority || 'normal'
      );
    } else {
      // Proxy other tools to AgentMux server
      result = await jsonRpcCall(config, 'tools/call', {
        name,
        arguments: args || {},
      });
    }

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
  console.error('[AgentMux MCP] Starting...');
  console.error(`[AgentMux MCP] Agent ID: ${config.agentId}`);
  console.error(`[AgentMux MCP] Server URL: ${config.url}`);
  console.error(`[AgentMux MCP] Auth: ${config.token ? 'Bearer token configured' : 'NO TOKEN - requests will fail!'}`);

  if (!config.token) {
    console.error('[AgentMux MCP] WARNING: Set AGENTMUX_TOKEN env var for authentication');
  }

  console.error('[AgentMux MCP] Creating stdio transport...');
  const transport = new StdioServerTransport();

  console.error('[AgentMux MCP] Connecting to MCP protocol...');
  await server.connect(transport);

  console.error('[AgentMux MCP] Ready - listening for requests');
}

main().catch((error) => {
  console.error('[AgentMux MCP] Fatal error:', error);
  process.exit(1);
});

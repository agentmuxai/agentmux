#!/usr/bin/env node

import { Server } from '@modelcontextprotocol/sdk/server/index.js';
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js';
import {
  CallToolRequestSchema,
  ListToolsRequestSchema,
} from '@modelcontextprotocol/sdk/types.js';
import { executeCommand, getAllowedCommands } from './executor.js';

// Create MCP server
const server = new Server(
  {
    name: 'agentmux-host-bridge',
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
        name: 'execute_command',
        description: 'Execute an allowed command on the Windows host system',
        inputSchema: {
          type: 'object',
          properties: {
            command: {
              type: 'string',
              enum: getAllowedCommands(),
              description: 'The command to execute (from allowed list)',
            },
            args: {
              type: 'array',
              items: { type: 'string' },
              description: 'Arguments to pass to the command',
            },
            cwd: {
              type: 'string',
              description: 'Working directory for command execution',
            },
          },
          required: ['command'],
        },
      },
      {
        name: 'list_allowed_commands',
        description: 'Get list of commands that can be executed on the host',
        inputSchema: {
          type: 'object',
          properties: {},
        },
      },
    ],
  };
});

// Handle tool calls
server.setRequestHandler(CallToolRequestSchema, async (request) => {
  const { name, arguments: args } = request.params;

  try {
    if (name === 'execute_command') {
      const result = await executeCommand(args as any);

      return {
        content: [
          {
            type: 'text',
            text: JSON.stringify(result, null, 2),
          },
        ],
        isError: !result.success,
      };
    }

    if (name === 'list_allowed_commands') {
      const commands = getAllowedCommands();

      return {
        content: [
          {
            type: 'text',
            text: JSON.stringify({ commands }, null, 2),
          },
        ],
      };
    }

    return {
      content: [
        {
          type: 'text',
          text: `Unknown tool: ${name}`,
        },
      ],
      isError: true,
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
  const transport = new StdioServerTransport();
  await server.connect(transport);

  console.error('[AgentMux Host Bridge] MCP server started');
  console.error('[AgentMux Host Bridge] Allowed commands:', getAllowedCommands().join(', '));
}

main().catch((error) => {
  console.error('Fatal error:', error);
  process.exit(1);
});

// Cleanup on exit
process.on('SIGTERM', () => {
  console.error('[AgentMux Host Bridge] Shutting down...');
  process.exit(0);
});

process.on('SIGINT', () => {
  console.error('[AgentMux Host Bridge] Shutting down...');
  process.exit(0);
});

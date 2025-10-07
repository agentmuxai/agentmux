/**
 * Built-in command handlers
 */

import { AgentMessage, MessageType } from '@agentmux/core';

export interface CommandHandler {
  pattern: RegExp;
  handler: (message: AgentMessage, bus: any) => Promise<void>;
  description: string;
}

export const builtInCommands: CommandHandler[] = [
  {
    pattern: /^(whoami|identify|who are you\??|agent id\??)$/i,
    description: 'Request agent to identify itself',
    handler: async (message, bus) => {
      // Respond with our identity
      await bus.send(
        message.from.id,
        MessageType.RESPONSE,
        {
          command: 'whoami',
          identity: bus.identity,
          response: `I am ${bus.identity.name} (${bus.identity.id})`,
          workspace: bus.identity.workspace,
          pid: bus.identity.pid,
          uptime: Date.now() - bus.identity.startedAt,
        },
        message.id
      );
    },
  },
  {
    pattern: /^ping$/i,
    description: 'Ping/pong test',
    handler: async (message, bus) => {
      await bus.send(
        message.from.id,
        MessageType.RESPONSE,
        {
          command: 'ping',
          response: 'pong',
          latency: Date.now() - message.timestamp,
        },
        message.id
      );
    },
  },
  {
    pattern: /^status$/i,
    description: 'Request agent status',
    handler: async (message, bus) => {
      await bus.send(
        message.from.id,
        MessageType.STATUS,
        {
          agent: bus.identity.name,
          status: 'active',
          uptime: Date.now() - bus.identity.startedAt,
          messagesReceived: bus.stats?.received || 0,
          messagesSent: bus.stats?.sent || 0,
        },
        message.id
      );
    },
  },
];

export function findCommandHandler(text: string): CommandHandler | undefined {
  return builtInCommands.find(cmd => cmd.pattern.test(text));
}

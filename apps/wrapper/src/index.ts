#!/usr/bin/env node

import { Command } from 'commander';
import { ClaudeWrapper } from './wrappers/claude';
import { AIWrapper } from './types';

const program = new Command();

program
  .name('agentmux-wrap')
  .description('Wrap AI CLI with reactive agent communication')
  .version('0.1.0');

program
  .argument('<cli>', 'CLI to wrap (claude, gemini, gpt, etc.)')
  .option('-a, --agent-id <id>', 'Agent ID (defaults to AGENT_ID env var or AgentX)')
  .option('-m, --messages-dir <dir>', 'Custom messages directory')
  .option('--debug', 'Enable debug logging')
  .action(async (cli: string, options: any) => {
    let wrapper: AIWrapper;

    // Select wrapper based on CLI
    switch (cli.toLowerCase()) {
      case 'claude':
        wrapper = new ClaudeWrapper({
          agentId: options.agentId,
          messagesDir: options.messagesDir,
          debug: options.debug
        });
        break;

      // Future: Add more CLI wrappers
      // case 'gemini':
      //   wrapper = new GeminiWrapper(options);
      //   break;
      // case 'gpt':
      //   wrapper = new GPTWrapper(options);
      //   break;

      default:
        console.error(`Unsupported CLI: ${cli}`);
        console.error('Supported CLIs: claude');
        console.error('Coming soon: gemini, gpt, cursor');
        process.exit(1);
    }

    // Start wrapper
    try {
      await wrapper.start();

      // Setup cleanup handlers
      const cleanup = () => {
        wrapper.stop();
        process.exit(0);
      };

      process.on('SIGINT', cleanup);
      process.on('SIGTERM', cleanup);
      process.on('exit', () => wrapper.stop());

    } catch (error) {
      console.error('Failed to start wrapper:', error);
      process.exit(1);
    }
  });

// Only parse if run directly as binary, not when imported as module
if (require.main === module) {
  program.parse();
}

export { ClaudeWrapper } from './wrappers/claude';
export { BaseWrapper } from './wrappers/base';
export { MessageWatcher } from './watcher';
export * from './types';

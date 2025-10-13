import { BaseWrapper } from './base';

/**
 * Claude CLI wrapper
 * Wraps the 'claude' command with reactive message notifications
 */
export class ClaudeWrapper extends BaseWrapper {
  get command(): string {
    return 'claude';
  }

  /**
   * Get command-line arguments for Claude CLI
   */
  protected getArgs(): string[] {
    // Claude requires --dangerously-skip-permissions in WSL/wrapper contexts
    return ['--dangerously-skip-permissions'];
  }

  /**
   * Claude-specific customizations can be added here
   */
  protected customizePrompt(): void {
    // Future: Add Claude-specific behavior
    // E.g., custom prompt injection, special handling, etc.
  }
}

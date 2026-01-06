import { exec } from 'child_process';
import { promisify } from 'util';

const execAsync = promisify(exec);

/**
 * Allowed commands that can be executed on the host.
 * Security: Strict allowlist to prevent arbitrary command execution
 */
const ALLOWED_COMMANDS = {
  vscode: 'code',
  explorer: 'explorer',
  git: 'git',
  notepad: 'notepad',
  terminal: 'wt', // Windows Terminal
  powershell: 'powershell',
  cmd: 'cmd',
} as const;

export type AllowedCommand = keyof typeof ALLOWED_COMMANDS;

export interface ExecuteCommandParams {
  command: AllowedCommand;
  args?: string[];
  cwd?: string;
}

export interface ExecuteCommandResult {
  success: boolean;
  stdout?: string;
  stderr?: string;
  error?: string;
}

/**
 * Execute an allowed command on the host system
 */
export async function executeCommand(
  params: ExecuteCommandParams
): Promise<ExecuteCommandResult> {
  const { command, args = [], cwd } = params;

  // Validate command is allowed
  const executable = ALLOWED_COMMANDS[command];
  if (!executable) {
    return {
      success: false,
      error: `Command '${command}' is not allowed. Allowed commands: ${Object.keys(ALLOWED_COMMANDS).join(', ')}`,
    };
  }

  // Build command string
  const cmdArgs = args.join(' ');
  const fullCommand = cmdArgs ? `${executable} ${cmdArgs}` : executable;

  try {
    const { stdout, stderr } = await execAsync(fullCommand, {
      cwd: cwd || process.cwd(),
      timeout: 30000, // 30 second timeout
      maxBuffer: 1024 * 1024, // 1MB max output
    });

    return {
      success: true,
      stdout: stdout.trim(),
      stderr: stderr.trim(),
    };
  } catch (error: any) {
    return {
      success: false,
      stdout: error.stdout?.trim(),
      stderr: error.stderr?.trim(),
      error: error.message,
    };
  }
}

/**
 * Get list of allowed commands
 */
export function getAllowedCommands(): string[] {
  return Object.keys(ALLOWED_COMMANDS);
}

import { Component, createSignal, onMount, Show } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import EmbeddedTerminal from './EmbeddedTerminal';

interface Agent {
  instanceName: string;
  pid: number;
  wsPort: number;
  status: string;
}

const TerminalWorkspace: Component = () => {
  const [agent, setAgent] = createSignal<Agent | null>(null);
  const [isLoading, setIsLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);

  const autoSpawnAgent = async () => {
    try {
      setIsLoading(true);
      setError(null);

      // Check if any Claude instances already exist
      const existingAgents: Agent[] = await invoke('list_claude_instances');

      if (existingAgents.length > 0) {
        // Use the first existing agent
        console.log('[TerminalWorkspace] Found existing agent:', existingAgents[0].instanceName);
        setAgent(existingAgents[0]);
      } else {
        // Auto-spawn a new Claude agent in the current directory
        console.log('[TerminalWorkspace] No existing agents, spawning new one...');

        // Use current directory or a default workspace
        const workspacePath = await invoke<string>('get_current_directory').catch(() => '.');

        const newAgent: Agent = await invoke('spawn_embedded_claude', {
          instanceName: 'Claude-1',
          workspacePath: workspacePath
        });

        console.log('[TerminalWorkspace] Claude agent spawned:', newAgent.instanceName);
        setAgent(newAgent);
      }
    } catch (err: any) {
      console.error('[TerminalWorkspace] Failed to spawn agent:', err);
      setError(err.toString());
    } finally {
      setIsLoading(false);
    }
  };

  onMount(() => {
    autoSpawnAgent();
  });

  return (
    <div class="terminal-workspace">
      <Show when={isLoading()}>
        <div class="terminal-loading">
          <div class="loading-spinner"></div>
          <p>Starting Claude agent...</p>
        </div>
      </Show>

      <Show when={error()}>
        <div class="terminal-error">
          <h3>Failed to start Claude agent</h3>
          <p>{error()}</p>
          <button class="primary" onClick={autoSpawnAgent}>
            Retry
          </button>
        </div>
      </Show>

      <Show when={agent() && !isLoading()}>
        {(currentAgent) => (
          <div class="terminal-pane">
            <EmbeddedTerminal
              instanceName={currentAgent().instanceName}
              wsPort={currentAgent().wsPort}
            />
          </div>
        )}
      </Show>
    </div>
  );
};

export default TerminalWorkspace;

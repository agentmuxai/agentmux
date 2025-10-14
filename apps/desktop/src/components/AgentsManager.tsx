import { Component, createSignal, createMemo, onMount, onCleanup, For, Show } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import SimpleTerminal from './SimpleTerminal';

interface Agent {
  instanceName: string;
  pid: number;
  wsPort: number;
  status: string;
  startedAt?: number;
}

interface LegacyAgent {
  agentId: string;
  status: string;
  pid?: number;
  startedAt?: number;
  messagesReceived?: number;
  outputLength?: number;
}

const AgentsManager: Component = () => {
  const [agents, setAgents] = createSignal<Agent[]>([]);
  const [selectedAgent, setSelectedAgent] = createSignal<string | null>(null);
  const [agentOutput, setAgentOutput] = createSignal('');
  const [newAgentId, setNewAgentId] = createSignal('');
  const [newAgentCommand, setNewAgentCommand] = createSignal('claude');
  const [error, setError] = createSignal<string | null>(null);
  const [isSpawning, setIsSpawning] = createSignal(false);

  // Memoize selected agent to prevent SimpleTerminal recreation
  const selectedAgentData = createMemo(() => {
    const name = selectedAgent();
    if (!name) return null;
    return agents().find(a => a.instanceName === name) || null;
  });

  let refreshInterval: number;
  let outputInterval: number;

  const loadAgents = async () => {
    try {
      console.log('[AgentsManager] Loading Claude instances...');
      const agentsList: Agent[] = await invoke('list_claude_instances');
      console.log('[AgentsManager] Loaded agents:', agentsList);
      setAgents(agentsList);
    } catch (err: any) {
      console.error('[AgentsManager] Failed to load agents:', err);
    }
  };

  const loadAgentOutput = async () => {
    const agentId = selectedAgent();
    if (!agentId) return;

    try {
      const output: string = await invoke('get_agent_output', { agentId });
      setAgentOutput(output);
    } catch (err: any) {
      console.error('Failed to load output:', err);
    }
  };

  const spawnAgent = async () => {
    const instanceName = newAgentId().trim();

    console.log('[AgentsManager] spawnAgent called with name:', instanceName);

    if (!instanceName) {
      console.warn('[AgentsManager] Instance name is empty');
      setError('Instance name is required');
      return;
    }

    setIsSpawning(true);
    setError(null);

    try {
      console.log('[AgentsManager] Invoking spawn_embedded_claude...');
      const result: Agent = await invoke('spawn_embedded_claude', {
        instanceName
      });

      console.log('[AgentsManager] Embedded Claude spawned successfully:', result);

      // Add to agents list
      setAgents([...agents(), result]);
      console.log('[AgentsManager] Updated agents list, new count:', agents().length + 1);

      // Select the new agent
      setSelectedAgent(instanceName);
      console.log('[AgentsManager] Selected new agent:', instanceName);

      // Clear inputs
      setNewAgentId('');
      setNewAgentCommand('claude');
      console.log('[AgentsManager] Cleared spawn form');
    } catch (err: any) {
      console.error('[AgentsManager] Failed to spawn instance:', err);
      setError(err.toString());
    } finally {
      setIsSpawning(false);
      console.log('[AgentsManager] Spawn process completed');
    }
  };

  const selectAgent = (agentId: string) => {
    setSelectedAgent(agentId);
    loadAgentOutput();
  };

  const sendMessageToAgent = async (agentId: string, message?: string) => {
    const messageText = message || 'Hello! This is a test message from the Desktop app. Please acknowledge receipt.';

    try {
      await invoke('send_message', {
        to: agentId,
        message: messageText,
        priority: 'normal'
      });

      console.log(`Message sent to ${agentId}: ${messageText}`);

      // Show confirmation
      setError(null);
    } catch (err: any) {
      setError(`Failed to send message: ${err.toString()}`);
      console.error('Failed to send message:', err);
    }
  };

  const formatUptime = (startedAt?: number) => {
    if (!startedAt) return 'N/A';
    const seconds = Math.floor((Date.now() - startedAt) / 1000);
    const minutes = Math.floor(seconds / 60);
    const hours = Math.floor(minutes / 60);

    if (hours > 0) return `${hours}h ${minutes % 60}m`;
    if (minutes > 0) return `${minutes}m ${seconds % 60}s`;
    return `${seconds}s`;
  };

  onMount(() => {
    loadAgents();
    refreshInterval = window.setInterval(loadAgents, 2000);
    outputInterval = window.setInterval(loadAgentOutput, 1000);
  });

  onCleanup(() => {
    if (refreshInterval) clearInterval(refreshInterval);
    if (outputInterval) clearInterval(outputInterval);
  });

  return (
    <div class="agents-manager">
      {/* Spawn New Agent */}
      <div class="card">
        <h2>🚀 Spawn New Agent</h2>

        {error() && (
          <div style={{ color: '#ef5350', 'margin-bottom': '1rem', 'font-size': '0.9rem' }}>
            Error: {error()}
          </div>
        )}

        <div class="form-grid">
          <div>
            <label>Agent ID:</label>
            <input
              type="text"
              placeholder="Agent2"
              value={newAgentId()}
              onInput={(e) => setNewAgentId(e.currentTarget.value)}
              disabled={isSpawning()}
            />
          </div>

          <div>
            <label>CLI Command:</label>
            <input
              type="text"
              placeholder="claude"
              value={newAgentCommand()}
              onInput={(e) => setNewAgentCommand(e.currentTarget.value)}
              disabled={isSpawning()}
            />
          </div>
        </div>

        <button
          class="primary"
          onClick={spawnAgent}
          disabled={isSpawning() || !newAgentId().trim()}
        >
          {isSpawning() ? '⏳ Spawning...' : '▶️ Spawn Agent'}
        </button>

        <p style={{ color: '#999', 'font-size': '0.85rem', 'margin-top': '0.5rem' }}>
          Agent will run Claude CLI and respond to messages reactively
        </p>
      </div>

      {/* Agent List */}
      <div class="card">
        <h2>🤖 Active Agents ({agents().length})</h2>

        <Show
          when={agents().length > 0}
          fallback={
            <p style={{ color: '#999' }}>
              No agents running. Spawn an agent above to get started.
            </p>
          }
        >
          <div class="agents-list">
            <For each={agents()}>
              {(agent) => (
                <div
                  class={`agent-card ${selectedAgent() === agent.instanceName ? 'selected' : ''}`}
                  onClick={() => selectAgent(agent.instanceName)}
                >
                  <div class="agent-header">
                    <span class="agent-id">
                      <span
                        class={`status-dot ${agent.status === 'running' ? 'online' : 'offline'}`}
                      ></span>
                      {agent.instanceName}
                    </span>
                    <span class="agent-pid">
                      PID: {agent.pid || 'N/A'}
                    </span>
                  </div>

                  <div class="agent-stats">
                    <div class="stat">
                      <span class="label">Status:</span>
                      <span class="value">{agent.status}</span>
                    </div>
                    <div class="stat">
                      <span class="label">WebSocket:</span>
                      <span class="value">:{agent.wsPort}</span>
                    </div>
                    <div class="stat">
                      <span class="label">Uptime:</span>
                      <span class="value">{formatUptime(agent.startedAt)}</span>
                    </div>
                  </div>
                </div>
              )}
            </For>
          </div>
        </Show>
      </div>

      {/* Embedded Terminal */}
      <Show when={selectedAgentData()}>
        {(agent) => (
          <div class="card">
            <h2>💻 Interactive Terminal: {agent().instanceName}</h2>
            <SimpleTerminal
              instanceName={agent().instanceName}
              wsPort={agent().wsPort}
            />
          </div>
        )}
      </Show>
    </div>
  );
};

export default AgentsManager;

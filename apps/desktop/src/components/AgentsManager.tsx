import { Component, createSignal, onMount, onCleanup, For, Show } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';

interface Agent {
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

  let refreshInterval: number;
  let outputInterval: number;

  const loadAgents = async () => {
    try {
      const agentsList: Agent[] = await invoke('list_agents');
      setAgents(agentsList);
    } catch (err: any) {
      console.error('Failed to load agents:', err);
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
    const agentId = newAgentId().trim();
    const cliCommand = newAgentCommand().trim() || 'claude';

    if (!agentId) {
      setError('Agent ID is required');
      return;
    }

    setIsSpawning(true);
    setError(null);

    try {
      const result = await invoke('spawn_agent', {
        agentId,
        cliCommand
      });

      console.log('Agent spawned:', result);

      // Clear inputs
      setNewAgentId('');
      setNewAgentCommand('claude');

      // Refresh list
      setTimeout(() => loadAgents(), 500);
    } catch (err: any) {
      setError(err.toString());
      console.error('Failed to spawn agent:', err);
    } finally {
      setIsSpawning(false);
    }
  };

  const selectAgent = (agentId: string) => {
    setSelectedAgent(agentId);
    loadAgentOutput();
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
                  class={`agent-card ${selectedAgent() === agent.agentId ? 'selected' : ''}`}
                  onClick={() => selectAgent(agent.agentId)}
                >
                  <div class="agent-header">
                    <span class="agent-id">
                      <span
                        class={`status-dot ${agent.status === 'running' ? 'online' : 'offline'}`}
                      ></span>
                      {agent.agentId}
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
                      <span class="label">Uptime:</span>
                      <span class="value">{formatUptime(agent.startedAt)}</span>
                    </div>
                    <div class="stat">
                      <span class="label">Messages:</span>
                      <span class="value">{agent.messagesReceived || 0}</span>
                    </div>
                    <div class="stat">
                      <span class="label">Output:</span>
                      <span class="value">{agent.outputLength || 0} bytes</span>
                    </div>
                  </div>
                </div>
              )}
            </For>
          </div>
        </Show>
      </div>

      {/* Agent Output Viewer */}
      <Show when={selectedAgent()}>
        <div class="card">
          <h2>📺 Live Output: {selectedAgent()}</h2>

          <div class="output-viewer">
            <pre class="output-content">{agentOutput() || 'Waiting for output...'}</pre>
          </div>

          <div class="output-controls">
            <button onClick={() => setAgentOutput('')}>
              🗑️ Clear Display
            </button>
            <button onClick={loadAgentOutput}>
              🔄 Refresh
            </button>
          </div>
        </div>
      </Show>
    </div>
  );
};

export default AgentsManager;

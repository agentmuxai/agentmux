import { Component, createSignal, createMemo, onMount, onCleanup, For, Show } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
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
  const [workspacePath, setWorkspacePath] = createSignal('');
  const [agentLabel, setAgentLabel] = createSignal('');
  const [newAgentCommand, setNewAgentCommand] = createSignal('claude');
  const [error, setError] = createSignal<string | null>(null);
  const [isSpawning, setIsSpawning] = createSignal(false);

  // Auto-suggest agent label from workspace path
  const suggestedLabel = createMemo(() => {
    const path = workspacePath();
    if (!path) return '';
    // Extract folder name from path as suggested label
    const folderName = path.split(/[/\\]/).filter(Boolean).pop() || '';
    return folderName;
  });

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

  const browseWorkspace = async () => {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: 'Select Workspace Directory for Agent'
      });

      if (selected && typeof selected === 'string') {
        setWorkspacePath(selected);
        // Auto-fill label if empty
        if (!agentLabel()) {
          setAgentLabel(suggestedLabel());
        }
        setError(null);
      }
    } catch (err: any) {
      console.error('[AgentsManager] Failed to open file browser:', err);
      setError('Failed to open file browser: ' + err);
    }
  };

  const spawnAgent = async () => {
    const label = agentLabel().trim() || suggestedLabel();
    const workspace = workspacePath().trim();

    console.log('[AgentsManager] spawnAgent called with label:', label, 'workspace:', workspace);

    if (!workspace) {
      console.warn('[AgentsManager] Workspace path is empty');
      setError('Workspace path is required');
      return;
    }

    if (!label) {
      console.warn('[AgentsManager] Agent label is empty');
      setError('Agent label is required (used for UI identification only)');
      return;
    }

    setIsSpawning(true);
    setError(null);

    try {
      console.log('[AgentsManager] Invoking spawn_embedded_claude...');
      const result: Agent = await invoke('spawn_embedded_claude', {
        instanceName: label, // Label is used for UI identification only
        workspacePath: workspace
      });

      console.log('[AgentsManager] Embedded Claude spawned successfully:', result);

      // Add to agents list
      setAgents([...agents(), result]);
      console.log('[AgentsManager] Updated agents list, new count:', agents().length + 1);

      // Select the new agent
      setSelectedAgent(label);
      console.log('[AgentsManager] Selected new agent:', label);

      // Clear inputs
      setWorkspacePath('');
      setAgentLabel('');
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

  onMount(async () => {
    // Initial load
    await loadAgents();

    // Set up event listeners for reactive updates
    const unlisteners: UnlistenFn[] = [];

    // Listen for agent_spawned events
    unlisteners.push(await listen('agent_spawned', (event) => {
      console.log('[AgentsManager] Received agent_spawned event:', event.payload);
      const payload = event.payload as {
        instance_name: string;
        pid: number;
        ws_port: number;
        status: string;
      };

      // Check if agent already exists (avoid duplicates)
      const exists = agents().some(a => a.instanceName === payload.instance_name);
      if (!exists) {
        const newAgent: Agent = {
          instanceName: payload.instance_name,
          pid: payload.pid,
          wsPort: payload.ws_port,
          status: payload.status
        };
        setAgents([...agents(), newAgent]);
        console.log('[AgentsManager] Added new agent from event:', payload.instance_name);
      }
    }));

    // Polling fallback for reconciliation (increased from 2s to 5s)
    refreshInterval = window.setInterval(async () => {
      const agentsList: Agent[] = await invoke('list_claude_instances');
      // Only update if counts differ (prevents unnecessary re-renders)
      if (agentsList.length !== agents().length) {
        console.log('[AgentsManager] Polling detected agent count change, updating list');
        setAgents(agentsList);
      }
    }, 5000);

    outputInterval = window.setInterval(loadAgentOutput, 1000);

    // Cleanup event listeners
    onCleanup(() => {
      unlisteners.forEach(fn => fn());
      if (refreshInterval) clearInterval(refreshInterval);
      if (outputInterval) clearInterval(outputInterval);
    });
  });

  return (
    <div class="agents-manager">
      {/* Spawn New Agent */}
      <div class="card">
        <h2>🚀 Spawn New Agent</h2>

        {error() && (
          <div style={{ color: '#ef5350', 'margin-bottom': '0.5rem', 'font-size': '0.9rem' }}>
            Error: {error()}
          </div>
        )}

        <div class="form-grid">
          <div>
            <label>Workspace Path:</label>
            <div style={{ display: 'flex', gap: '0.5rem' }}>
              <input
                type="text"
                placeholder="Select workspace directory..."
                value={workspacePath()}
                onInput={(e) => setWorkspacePath(e.currentTarget.value)}
                disabled={isSpawning()}
                style={{ flex: 1 }}
              />
              <button
                onClick={browseWorkspace}
                disabled={isSpawning()}
                style={{
                  background: '#2a2a2a',
                  color: '#e0e0e0',
                  border: '1px solid #3a3a3a',
                  padding: '0.375rem 0.75rem',
                  'border-radius': '6px',
                  cursor: isSpawning() ? 'not-allowed' : 'pointer',
                  'font-weight': '500'
                }}
              >
                📁 Browse
              </button>
            </div>
            <div style={{
              color: '#666',
              'font-size': '0.8rem',
              'margin-top': '0.25rem'
            }}>
              Agent will spawn in this directory (agent name inferred from path)
            </div>
          </div>

          <div>
            <label>Agent Label (UI only):</label>
            <input
              type="text"
              placeholder={suggestedLabel() || "MyAgent"}
              value={agentLabel()}
              onInput={(e) => setAgentLabel(e.currentTarget.value)}
              disabled={isSpawning()}
            />
            <div style={{
              color: '#666',
              'font-size': '0.8rem',
              'margin-top': '0.25rem'
            }}>
              Optional shortcut identifier (not visible to agent)
            </div>
          </div>
        </div>

        <button
          class="primary"
          onClick={spawnAgent}
          disabled={isSpawning() || !workspacePath().trim()}
        >
          {isSpawning() ? '⏳ Spawning...' : '▶️ Spawn Agent'}
        </button>

        <p style={{ color: '#999', 'font-size': '0.85rem', 'margin-top': '0.25rem' }}>
          💡 Agent spawns in selected workspace. Agent identity is determined by workspace path (e.g., WebProjects1 → Agent1).
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

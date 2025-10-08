import { Component, For, createSignal, onMount, onCleanup } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';

interface Agent {
  id: string;
  name: string;
  workspace: string;
  status: string;
  connected_at: number;
  uptime: number;
  messages_sent: number;
  messages_received: number;
}

const AgentList: Component = () => {
  const [agents, setAgents] = createSignal<Agent[]>([]);

  const formatUptime = (seconds: number): string => {
    const hours = Math.floor(seconds / 3600);
    const minutes = Math.floor((seconds % 3600) / 60);
    if (hours > 0) {
      return `${hours}h ${minutes}m`;
    }
    return `${minutes}m`;
  };

  const updateAgents = async () => {
    try {
      const agentList = await invoke<Agent[]>('get_connected_agents');
      setAgents(agentList);
    } catch (err) {
      console.error('Failed to get agents:', err);
    }
  };

  let intervalId: number;

  onMount(() => {
    updateAgents();
    intervalId = window.setInterval(updateAgents, 2000);
  });

  onCleanup(() => {
    if (intervalId) {
      clearInterval(intervalId);
    }
  });

  return (
    <div>
      <div class="card">
        <h2>Agent Registry ({agents().length} total)</h2>

        {agents().length === 0 ? (
          <p style={{ color: '#999' }}>
            No agents connected. Start the bus and connect agents to see them here.
          </p>
        ) : (
          <For each={agents()}>
            {(agent) => (
              <div class="agent-item">
                <div class="info">
                  <div class="name">
                    <span class={`status-dot ${agent.status}`}></span>
                    {agent.name} ({agent.id})
                  </div>
                  <div class="workspace">
                    {agent.workspace} |
                    ↑ {agent.messages_sent} ↓ {agent.messages_received} |
                    Uptime: {formatUptime(agent.uptime)}
                  </div>
                </div>
                <button
                  class="danger"
                  style={{ padding: '0.5rem 1rem', 'font-size': '0.85rem' }}
                >
                  Disconnect
                </button>
              </div>
            )}
          </For>
        )}
      </div>

      <div class="card">
        <h2>Connection Statistics</h2>
        <p style={{ color: '#999' }}>Detailed statistics will be shown here once agents connect.</p>
      </div>
    </div>
  );
};

export default AgentList;

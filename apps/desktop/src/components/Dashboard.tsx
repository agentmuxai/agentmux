import { Component, createSignal, onMount, onCleanup } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

const Dashboard: Component = () => {
  const [busRunning, setBusRunning] = createSignal(false);
  const [connectedAgents, setConnectedAgents] = createSignal(0);
  const [messagesPerSec, setMessagesPerSec] = createSignal(0);
  const [totalMessages, setTotalMessages] = createSignal(0);
  const [error, setError] = createSignal<string | null>(null);

  // Poll bus status every 2 seconds
  let intervalId: number;
  let unlistenCommand: UnlistenFn | undefined;

  const updateStatus = async () => {
    try {
      const status: any = await invoke('get_bus_status');
      setBusRunning(status.running);
      setConnectedAgents(status.agents_connected);
      setMessagesPerSec(status.messages_per_second);
      setTotalMessages(status.total_messages || 0);
      setError(null);
    } catch (err) {
      console.error('Failed to get bus status:', err);
    }
  };

  const handleStartBus = async () => {
    try {
      setError(null);
      const result = await invoke<string>('start_bus', {
        config: {
          host: '127.0.0.1',
          port: 8765,
          max_agents: 50
        }
      });
      console.log(result);
      await updateStatus();
    } catch (err: any) {
      setError(err.toString());
      console.error('Failed to start bus:', err);
    }
  };

  const handleStopBus = async () => {
    try {
      setError(null);
      const result = await invoke<string>('stop_bus');
      console.log(result);
      await updateStatus();
    } catch (err: any) {
      setError(err.toString());
      console.error('Failed to stop bus:', err);
    }
  };

  const handleSpawnClaude = async () => {
    try {
      const instanceName = prompt('Enter instance name (e.g., Alice, Bob):');
      if (!instanceName) return;

      setError(null);
      await invoke('spawn_claude_instance', { instanceName });
      console.log(`Spawned Claude instance: ${instanceName}`);
    } catch (err: any) {
      setError(err.toString());
      console.error('Failed to spawn Claude instance:', err);
    }
  };

  onMount(async () => {
    updateStatus();
    intervalId = window.setInterval(updateStatus, 2000);

    // Start command watcher for CLI integration
    try {
      await invoke('start_command_watcher');
      console.log('Command watcher started');
    } catch (err) {
      console.error('Failed to start command watcher:', err);
    }

    // Listen for CLI commands
    unlistenCommand = await listen('cli_command', async (event: any) => {
      const command = event.payload;
      console.log('CLI command received:', command);

      try {
        switch (command.command) {
          case 'start_bus':
            await handleStartBus();
            break;
          case 'stop_bus':
            await handleStopBus();
            break;
          default:
            console.warn('Unknown CLI command:', command.command);
        }
      } catch (err) {
        console.error('Error executing CLI command:', err);
      }
    });
  });

  onCleanup(() => {
    if (intervalId) {
      clearInterval(intervalId);
    }
    if (unlistenCommand) {
      unlistenCommand();
    }
  });

  return (
    <div>
      <div class="card">
        <h2>Server Bus Control</h2>
        <div class="bus-status">
          <span class={`status-dot ${busRunning() ? 'online' : 'offline'}`}></span>
          <span>Status: {busRunning() ? 'Running' : 'Stopped'}</span>
        </div>
        {error() && (
          <div style={{ color: '#ef5350', 'margin-bottom': '1rem', 'font-size': '0.9rem' }}>
            Error: {error()}
          </div>
        )}
        <div style={{ display: 'flex', gap: '1rem' }}>
          <button class="primary" onClick={handleStartBus} disabled={busRunning()}>
            ▶️ Start Bus
          </button>
          <button class="danger" onClick={handleStopBus} disabled={!busRunning()}>
            ⏹️ Stop Bus
          </button>
        </div>
      </div>

      <div class="stats">
        <div class="stat-card">
          <div class="label">Connected Agents</div>
          <div class="value">{connectedAgents()}</div>
          <div class="change">{busRunning() ? 'Live' : 'Offline'}</div>
        </div>
        <div class="stat-card">
          <div class="label">Messages/sec</div>
          <div class="value">{messagesPerSec()}</div>
          <div class="change">{totalMessages()} total</div>
        </div>
        <div class="stat-card">
          <div class="label">Bus Status</div>
          <div class="value">{busRunning() ? '✓' : '✗'}</div>
          <div class="change">ws://localhost:8765</div>
        </div>
      </div>

      <div class="card">
        <h2>Recent Activity</h2>
        <p style={{ color: '#999' }}>
          {busRunning()
            ? 'Bus is running. Agents can connect at ws://localhost:8765/ws'
            : 'Start the bus to begin monitoring agents.'}
        </p>
      </div>

      <div class="card">
        <h2>🧪 Reactive Claude Demo</h2>
        <p style={{ color: '#999', 'margin-bottom': '1rem' }}>
          Launch wrapped Claude instances that can message each other reactively.
        </p>
        <div style={{ display: 'flex', gap: '1rem', 'flex-wrap': 'wrap' }}>
          <button class="primary" onClick={handleSpawnClaude}>
            🚀 Spawn Claude Instance
          </button>
        </div>
        <div style={{ 'margin-top': '1rem', color: '#666', 'font-size': '0.85rem' }}>
          <div>Each instance can send messages to other instances using MCP tools.</div>
          <div>Messages are delivered reactively without human intervention.</div>
        </div>
      </div>
    </div>
  );
};

export default Dashboard;

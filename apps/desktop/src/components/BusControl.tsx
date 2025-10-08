import { Component, createSignal } from 'solid-js';

const BusControl: Component = () => {
  const [host, setHost] = createSignal('localhost');
  const [port, setPort] = createSignal('8765');
  const [maxAgents, setMaxAgents] = createSignal('50');

  return (
    <div>
      <div class="card">
        <h2>Bus Configuration</h2>

        <div class="config-grid">
          <label>Protocol:</label>
          <select style="background: #1a1a1a; border: 1px solid #3a3a3a; color: #e0e0e0; padding: 0.5rem; border-radius: 4px;">
            <option>WebSocket</option>
          </select>

          <label>Host:</label>
          <input
            type="text"
            value={host()}
            onInput={(e) => setHost(e.currentTarget.value)}
          />

          <label>Port:</label>
          <input
            type="text"
            value={port()}
            onInput={(e) => setPort(e.currentTarget.value)}
          />

          <label>Max Agents:</label>
          <input
            type="text"
            value={maxAgents()}
            onInput={(e) => setMaxAgents(e.currentTarget.value)}
          />
        </div>

        <div style={{ display: 'flex', gap: '1rem' }}>
          <button class="primary">💾 Save Config</button>
          <button class="primary">🔄 Restart Bus</button>
        </div>
      </div>

      <div class="card">
        <h2>Connection Info</h2>
        <div style={{ 'font-family': 'monospace', 'font-size': '0.9rem', color: '#999' }}>
          <p>WebSocket URL: ws://{host()}:{port()}</p>
          <p>HTTP Health: http://{host()}:{port()}/health</p>
          <p>Metrics: http://{host()}:{port()}/metrics</p>
        </div>
      </div>

      <div class="card">
        <h2>Performance Metrics</h2>
        <p style={{ color: '#999' }}>Charts will be displayed here once the bus is running.</p>
      </div>
    </div>
  );
};

export default BusControl;

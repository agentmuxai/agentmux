import { Component, createSignal } from 'solid-js';
import Dashboard from './components/Dashboard';
import BusControl from './components/BusControl';
import AgentsManager from './components/AgentsManager';
import MessageStream from './components/MessageStream';
import { DebugConsole } from './components/DebugConsole';

const App: Component = () => {
  const [activeTab, setActiveTab] = createSignal<'dashboard' | 'bus' | 'agents' | 'messages'>('dashboard');

  return (
    <div class="app">
      <header class="app-header">
        <h1>🤖 AgentMux Desktop</h1>
        <div class="tabs">
          <button
            class={activeTab() === 'dashboard' ? 'tab active' : 'tab'}
            onClick={() => setActiveTab('dashboard')}
          >
            🚀 Dashboard
          </button>
          <button
            class={activeTab() === 'bus' ? 'tab active' : 'tab'}
            onClick={() => setActiveTab('bus')}
          >
            🔌 Bus
          </button>
          <button
            class={activeTab() === 'agents' ? 'tab active' : 'tab'}
            onClick={() => setActiveTab('agents')}
          >
            🤖 Agents
          </button>
          <button
            class={activeTab() === 'messages' ? 'tab active' : 'tab'}
            onClick={() => setActiveTab('messages')}
          >
            💬 Messages
          </button>
        </div>
      </header>

      <main class="app-content">
        {activeTab() === 'dashboard' && <Dashboard />}
        {activeTab() === 'bus' && <BusControl />}
        {activeTab() === 'agents' && <AgentsManager />}
        {activeTab() === 'messages' && <MessageStream />}
      </main>

      <footer class="app-footer">
        <span>AgentMux v0.2.6</span>
        <span>|</span>
        <span>Built: 2025-10-13 4:51 AM PT</span>
        <span>|</span>
        <span>Status: Ready</span>
      </footer>

      <DebugConsole />
    </div>
  );
};

export default App;

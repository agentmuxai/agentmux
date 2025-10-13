import { Component, createSignal } from 'solid-js';
import Dashboard from './components/Dashboard';
import BusControl from './components/BusControl';
import AgentList from './components/AgentList';
import MessageStream from './components/MessageStream';
import AgentsManager from './components/AgentsManager';

const App: Component = () => {
  const [activeTab, setActiveTab] = createSignal<'dashboard' | 'bus' | 'agents' | 'messages' | 'agents-manager'>('dashboard');

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
          <button
            class={activeTab() === 'agents-manager' ? 'tab active' : 'tab'}
            onClick={() => setActiveTab('agents-manager')}
          >
            🧠 CLI Agents
          </button>
        </div>
      </header>

      <main class="app-content">
        {activeTab() === 'dashboard' && <Dashboard />}
        {activeTab() === 'bus' && <BusControl />}
        {activeTab() === 'agents' && <AgentList />}
        {activeTab() === 'messages' && <MessageStream />}
        {activeTab() === 'agents-manager' && <AgentsManager />}
      </main>

      <footer class="app-footer">
        <span>AgentMux v0.1.0</span>
        <span>|</span>
        <span>Status: Ready</span>
      </footer>
    </div>
  );
};

export default App;

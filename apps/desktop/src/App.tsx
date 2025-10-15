import { Component, createSignal } from 'solid-js';
import Dashboard from './components/Dashboard';
import BusControl from './components/BusControl';
import AgentsManager from './components/AgentsManager';
import MessageStream from './components/MessageStream';
import { DebugConsole } from './components/DebugConsole';

const BUILD_TIME = '__BUILD_TIME__'; // Replaced at build time
const VERSION = '__VERSION__'; // Replaced at build time

const App: Component = () => {
  const [activeTab, setActiveTab] = createSignal<'dashboard' | 'bus' | 'agents' | 'messages'>('dashboard');

  return (
    <div class="app" data-testid="app-ready">
      <header class="app-header" data-testid="app-header">
        <h1 data-testid="app-title">🤖 AgentMux Desktop</h1>
        <div class="tabs" data-testid="tabs-container">
          <button
            class={activeTab() === 'dashboard' ? 'tab active' : 'tab'}
            onClick={() => setActiveTab('dashboard')}
            data-testid="tab-dashboard"
          >
            🚀 Dashboard
          </button>
          <button
            class={activeTab() === 'bus' ? 'tab active' : 'tab'}
            onClick={() => setActiveTab('bus')}
            data-testid="tab-bus"
          >
            🔌 Bus
          </button>
          <button
            class={activeTab() === 'agents' ? 'tab active' : 'tab'}
            onClick={() => setActiveTab('agents')}
            data-testid="tab-agents"
          >
            🤖 Agents
          </button>
          <button
            class={activeTab() === 'messages' ? 'tab active' : 'tab'}
            onClick={() => setActiveTab('messages')}
            data-testid="tab-messages"
          >
            💬 Messages
          </button>
        </div>
      </header>

      <main class="app-content" data-testid="app-content">
        {activeTab() === 'dashboard' && <Dashboard />}
        {activeTab() === 'bus' && <BusControl />}
        {activeTab() === 'agents' && <AgentsManager />}
        {activeTab() === 'messages' && <MessageStream />}
      </main>

      <footer class="app-footer" data-testid="status-bar">
        <span data-testid="app-version">AgentMux v{VERSION}</span>
        <span>|</span>
        <span data-testid="build-timestamp">Built: {BUILD_TIME}</span>
        <span>|</span>
        <span data-testid="app-status">Status: Ready</span>
      </footer>

      <DebugConsole />
    </div>
  );
};

export default App;

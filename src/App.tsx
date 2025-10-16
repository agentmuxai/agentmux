import { Component, createSignal } from 'solid-js';
import Menubar from './components/Menubar';
import PaneContainer from './components/PaneContainer';
import Modal from './components/Modal';
import Dashboard from './components/Dashboard';
import BusControl from './components/BusControl';
import AgentsManager from './components/AgentsManager';
import MessageStream from './components/MessageStream';
import { DebugConsole } from './components/DebugConsole';

const BUILD_TIME = '__BUILD_TIME__'; // Replaced at build time
const VERSION = '__VERSION__'; // Replaced at build time

type ModalView = 'dashboard' | 'bus' | 'agents' | 'messages' | null;

const App: Component = () => {
  const [activeModal, setActiveModal] = createSignal<ModalView>(null);
  const [paneCount, setPaneCount] = createSignal(1);

  const closeModal = () => setActiveModal(null);

  return (
    <div class="app app-terminal-first" data-testid="app-ready">
      <header class="app-header-minimal" data-testid="app-header">
        <Menubar
          onShowDashboard={() => setActiveModal('dashboard')}
          onShowBusInfo={() => setActiveModal('bus')}
          onShowAgentList={() => setActiveModal('agents')}
          onShowMessageStream={() => setActiveModal('messages')}
          paneCount={paneCount()}
        />
        <h1 class="app-title-minimal" data-testid="app-title">AgentMux Desktop</h1>
        <div class="app-status-minimal">
          <span data-testid="app-version">v{VERSION}</span>
          <span style={{ 'margin-left': '0.5rem', color: '#666' }}>
            ({paneCount()} {paneCount() === 1 ? 'pane' : 'panes'})
          </span>
        </div>
      </header>

      <main class="app-content-fullscreen" data-testid="app-content">
        <PaneContainer onPanesChange={setPaneCount} />
      </main>

      {/* Modals for management views */}
      <Modal
        isOpen={activeModal() === 'dashboard'}
        onClose={closeModal}
        title="Dashboard"
      >
        <Dashboard />
      </Modal>

      <Modal
        isOpen={activeModal() === 'bus'}
        onClose={closeModal}
        title="Bus Control"
      >
        <BusControl />
      </Modal>

      <Modal
        isOpen={activeModal() === 'agents'}
        onClose={closeModal}
        title="Agent Manager"
      >
        <AgentsManager />
      </Modal>

      <Modal
        isOpen={activeModal() === 'messages'}
        onClose={closeModal}
        title="Message Stream"
      >
        <MessageStream />
      </Modal>

      <DebugConsole />
    </div>
  );
};

export default App;

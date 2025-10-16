import { Component, createSignal, Show } from 'solid-js';

interface MenubarProps {
  onShowDashboard: () => void;
  onShowBusInfo: () => void;
  onShowAgentList: () => void;
  onShowMessageStream: () => void;
  paneCount?: number;
}

const Menubar: Component<MenubarProps> = (props) => {
  const [isOpen, setIsOpen] = createSignal(false);

  const toggleMenu = () => {
    setIsOpen(!isOpen());
  };

  const closeMenu = () => {
    setIsOpen(false);
  };

  const handleAction = (action: () => void) => {
    action();
    closeMenu();
  };

  return (
    <div class="menubar">
      <button
        class="menubar-toggle"
        onClick={toggleMenu}
        aria-label="Toggle menu"
        data-testid="menubar-toggle"
      >
        ☰
      </button>

      <Show when={isOpen()}>
        <div class="menubar-overlay" onClick={closeMenu} />
        <div class="menubar-dropdown" data-testid="menubar-dropdown">
          <div class="menubar-section">
            <div class="menubar-section-title">View</div>
            <button
              class="menubar-item"
              onClick={() => handleAction(props.onShowDashboard)}
              data-testid="menu-dashboard"
            >
              🚀 Dashboard
            </button>
          </div>

          <div class="menubar-divider" />

          <div class="menubar-section">
            <div class="menubar-section-title">Layout</div>
            <button
              class="menubar-item"
              onClick={() => handleAction(() => {
                (window as any).paneActions?.splitVertical();
              })}
              data-testid="menu-split-vertical"
            >
              ⬌ Split Vertical
            </button>
            <button
              class="menubar-item"
              onClick={() => handleAction(() => {
                (window as any).paneActions?.splitHorizontal();
              })}
              data-testid="menu-split-horizontal"
            >
              ⬍ Split Horizontal
            </button>
            <button
              class="menubar-item"
              onClick={() => handleAction(() => {
                (window as any).paneActions?.closeCurrentPane();
              })}
              disabled={!props.paneCount || props.paneCount <= 1}
              data-testid="menu-close-pane"
            >
              ✕ Close Current Pane
            </button>
            <button
              class="menubar-item"
              onClick={() => handleAction(() => {
                (window as any).paneActions?.resetToSingle();
              })}
              disabled={!props.paneCount || props.paneCount <= 1}
              data-testid="menu-reset-layout"
            >
              ▢ Reset to Single Pane
            </button>
          </div>

          <div class="menubar-divider" />

          <div class="menubar-section">
            <div class="menubar-section-title">Management</div>
            <button
              class="menubar-item"
              onClick={() => handleAction(props.onShowAgentList)}
              data-testid="menu-agents"
            >
              🤖 Agent List
            </button>
            <button
              class="menubar-item"
              onClick={() => handleAction(props.onShowBusInfo)}
              data-testid="menu-bus"
            >
              🔌 Bus Info
            </button>
            <button
              class="menubar-item"
              onClick={() => handleAction(props.onShowMessageStream)}
              data-testid="menu-messages"
            >
              💬 Message Stream
            </button>
          </div>

          <div class="menubar-divider" />

          <div class="menubar-section">
            <div class="menubar-section-title">Help</div>
            <button
              class="menubar-item"
              onClick={() => handleAction(() => {
                window.open('https://github.com/a5af/agentmux', '_blank');
              })}
              data-testid="menu-docs"
            >
              📚 Documentation
            </button>
            <button
              class="menubar-item"
              onClick={() => handleAction(() => {
                alert('AgentMux Desktop v0.3.24\nTerminal-first multi-agent workspace');
              })}
              data-testid="menu-about"
            >
              ℹ️ About
            </button>
          </div>
        </div>
      </Show>
    </div>
  );
};

export default Menubar;

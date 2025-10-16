import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@solidjs/testing-library';
import App from './App';

// Mock components
vi.mock('./components/Menubar', () => ({
  default: (props: any) => (
    <div data-testid="menubar">
      Menubar (panes: {props.paneCount})
    </div>
  ),
}));

vi.mock('./components/PaneContainer', () => ({
  default: (props: any) => {
    // Simulate pane count change
    setTimeout(() => props.onPanesChange?.(1), 0);
    return <div data-testid="pane-container">PaneContainer</div>;
  },
}));

vi.mock('./components/Modal', () => ({
  default: (props: any) => (
    props.isOpen ? (
      <div data-testid={`modal-${props.title}`}>
        Modal: {props.title}
        {props.children}
      </div>
    ) : null
  ),
}));

vi.mock('./components/Dashboard', () => ({
  default: () => <div data-testid="dashboard-content">Dashboard</div>,
}));

vi.mock('./components/BusControl', () => ({
  default: () => <div data-testid="bus-content">Bus Control</div>,
}));

vi.mock('./components/AgentsManager', () => ({
  default: () => <div data-testid="agents-content">Agents Manager</div>,
}));

vi.mock('./components/MessageStream', () => ({
  default: () => <div data-testid="messages-content">Message Stream</div>,
}));

vi.mock('./components/DebugConsole', () => ({
  DebugConsole: () => <div data-testid="debug-console">Debug Console</div>,
}));

describe('App', () => {
  it('renders with terminal-first layout', () => {
    render(() => <App />);

    expect(screen.getByTestId('app-ready')).toBeTruthy();
    expect(screen.getByTestId('app-header')).toBeTruthy();
    expect(screen.getByTestId('app-content')).toBeTruthy();
  });

  it('renders app title', () => {
    render(() => <App />);

    const title = screen.getByTestId('app-title');
    expect(title.textContent).toBe('AgentMux Desktop');
  });

  it('displays version', () => {
    render(() => <App />);

    const version = screen.getByTestId('app-version');
    // In test env, VERSION might be __VERSION__ placeholder
    expect(version.textContent).toMatch(/v(__VERSION__|\d+\.\d+\.\d+)/);
  });

  it('renders Menubar component', () => {
    render(() => <App />);

    expect(screen.getByTestId('menubar')).toBeTruthy();
  });

  it('renders PaneContainer component', () => {
    render(() => <App />);

    expect(screen.getByTestId('pane-container')).toBeTruthy();
  });

  it('passes pane count to Menubar', async () => {
    render(() => <App />);

    await vi.waitFor(() => {
      const menubar = screen.getByTestId('menubar');
      expect(menubar.textContent).toContain('panes: 1');
    });
  });

  it('displays pane count in header', async () => {
    render(() => <App />);

    await vi.waitFor(() => {
      const header = screen.getByTestId('app-header');
      expect(header.textContent).toMatch(/1 pane/);
    });
  });

  it('uses plural "panes" when count > 1', async () => {
    // Mock PaneContainer to report 2 panes
    vi.resetModules();
    vi.doMock('./components/PaneContainer', () => ({
      default: (props: any) => {
        setTimeout(() => props.onPanesChange?.(2), 0);
        return <div data-testid="pane-container">PaneContainer</div>;
      },
    }));

    const { default: AppWithMultiplePanes } = await import('./App');
    render(() => <AppWithMultiplePanes />);

    await vi.waitFor(() => {
      const header = screen.getByTestId('app-header');
      expect(header.textContent).toMatch(/2 panes/);
    });
  });

  it('has app-terminal-first class', () => {
    render(() => <App />);

    const app = screen.getByTestId('app-ready');
    expect(app.classList.contains('app-terminal-first')).toBe(true);
  });

  it('has minimal header styling', () => {
    render(() => <App />);

    const header = screen.getByTestId('app-header');
    expect(header.classList.contains('app-header-minimal')).toBe(true);
  });

  it('has fullscreen content area', () => {
    render(() => <App />);

    const content = screen.getByTestId('app-content');
    expect(content.classList.contains('app-content-fullscreen')).toBe(true);
  });

  it('renders DebugConsole', () => {
    render(() => <App />);

    expect(screen.getByTestId('debug-console')).toBeTruthy();
  });

  it('does not show modals by default', () => {
    render(() => <App />);

    expect(screen.queryByTestId('modal-Dashboard')).toBeFalsy();
    expect(screen.queryByTestId('modal-Bus Control')).toBeFalsy();
    expect(screen.queryByTestId('modal-Agent Manager')).toBeFalsy();
    expect(screen.queryByTestId('modal-Message Stream')).toBeFalsy();
  });
});

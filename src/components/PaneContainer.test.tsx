import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, waitFor } from '@solidjs/testing-library';
import { userEvent } from '@testing-library/user-event';
import PaneContainer from './PaneContainer';

// Mock Tauri invoke
const mockInvoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: any[]) => mockInvoke(...args),
}));

// Mock EmbeddedTerminal
vi.mock('./EmbeddedTerminal', () => ({
  default: (props: any) => (
    <div data-testid={`terminal-${props.instanceName}`}>
      Terminal: {props.instanceName} (port: {props.wsPort})
    </div>
  ),
}));

describe('PaneContainer', () => {
  beforeEach(() => {
    // Clear localStorage before each test
    localStorage.clear();
    mockInvoke.mockClear();

    // Mock list_claude_instances to return empty by default
    mockInvoke.mockImplementation((command: string) => {
      if (command === 'list_claude_instances') {
        return Promise.resolve([]);
      }
      if (command === 'spawn_embedded_claude') {
        return Promise.resolve({
          instanceName: 'Claude-1',
          pid: 12345,
          wsPort: 9001,
          status: 'running',
        });
      }
      return Promise.resolve();
    });
  });

  afterEach(() => {
    vi.clearAllTimers();
  });

  it('renders with single pane on initial load', async () => {
    render(() => <PaneContainer />);

    await waitFor(() => {
      const pane = screen.getByText(/Loading\.\.\.|Claude-1/);
      expect(pane).toBeTruthy();
    });
  });

  it('auto-spawns agent for first pane', async () => {
    render(() => <PaneContainer />);

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith('spawn_embedded_claude', {
        instanceName: 'Claude-1',
        workspacePath: '~',
      });
    }, { timeout: 3000 });
  });

  it('exposes paneActions on window object', () => {
    render(() => <PaneContainer />);

    expect((window as any).paneActions).toBeDefined();
    expect((window as any).paneActions.splitVertical).toBeInstanceOf(Function);
    expect((window as any).paneActions.splitHorizontal).toBeInstanceOf(Function);
    expect((window as any).paneActions.closeCurrentPane).toBeInstanceOf(Function);
    expect((window as any).paneActions.resetToSingle).toBeInstanceOf(Function);
    expect((window as any).paneActions.getPaneCount).toBeInstanceOf(Function);
  });

  it('splits pane vertically via window.paneActions', async () => {
    const onPanesChange = vi.fn();
    render(() => <PaneContainer onPanesChange={onPanesChange} />);

    // Wait for initial pane
    await waitFor(() => {
      expect((window as any).paneActions.getPaneCount()).toBe(1);
    });

    // Clear initial call
    onPanesChange.mockClear();

    // Split vertically
    (window as any).paneActions.splitVertical();

    await waitFor(() => {
      expect((window as any).paneActions.getPaneCount()).toBe(2);
    });

    // Verify callback was called with 2
    expect(onPanesChange).toHaveBeenCalledWith(2);
  });

  it('splits pane horizontally via window.paneActions', async () => {
    const onPanesChange = vi.fn();
    render(() => <PaneContainer onPanesChange={onPanesChange} />);

    await waitFor(() => {
      expect((window as any).paneActions.getPaneCount()).toBe(1);
    });

    // Clear initial call
    onPanesChange.mockClear();

    (window as any).paneActions.splitHorizontal();

    await waitFor(() => {
      expect((window as any).paneActions.getPaneCount()).toBe(2);
    });

    expect(onPanesChange).toHaveBeenCalledWith(2);
  });

  it('prevents closing last pane', async () => {
    render(() => <PaneContainer />);

    await waitFor(() => {
      expect((window as any).paneActions.getPaneCount()).toBe(1);
    });

    // Try to close the only pane
    (window as any).paneActions.closeCurrentPane();

    // Should still have 1 pane
    expect((window as any).paneActions.getPaneCount()).toBe(1);
  });

  it('allows closing pane when multiple exist', async () => {
    const onPanesChange = vi.fn();
    render(() => <PaneContainer onPanesChange={onPanesChange} />);

    await waitFor(() => {
      expect((window as any).paneActions.getPaneCount()).toBe(1);
    });

    // Add a pane
    (window as any).paneActions.splitVertical();

    await waitFor(() => {
      expect((window as any).paneActions.getPaneCount()).toBe(2);
    });

    // Clear previous calls
    onPanesChange.mockClear();

    // Close current pane
    (window as any).paneActions.closeCurrentPane();

    await waitFor(() => {
      expect((window as any).paneActions.getPaneCount()).toBe(1);
    });

    expect(onPanesChange).toHaveBeenCalledWith(1);
  });

  it('resets to single pane', async () => {
    const onPanesChange = vi.fn();
    render(() => <PaneContainer onPanesChange={onPanesChange} />);

    await waitFor(() => {
      expect((window as any).paneActions.getPaneCount()).toBe(1);
    });

    // Add multiple panes
    (window as any).paneActions.splitVertical();
    (window as any).paneActions.splitHorizontal();

    await waitFor(() => {
      expect((window as any).paneActions.getPaneCount()).toBeGreaterThan(1);
    });

    // Reset to single
    (window as any).paneActions.resetToSingle();

    await waitFor(() => {
      expect((window as any).paneActions.getPaneCount()).toBe(1);
      expect(onPanesChange).toHaveBeenCalledWith(1);
    });
  });

  describe('Session Persistence', () => {
    it('saves layout to localStorage', async () => {
      render(() => <PaneContainer />);

      await waitFor(() => {
        const stored = localStorage.getItem('agentmux.layout');
        expect(stored).toBeTruthy();
      }, { timeout: 3000 });

      const layout = JSON.parse(localStorage.getItem('agentmux.layout')!);
      expect(layout.panes).toHaveLength(1);
      expect(layout.orientation).toBe('vertical');
      expect(layout.activePaneId).toBe('pane-1');
    });

    it('saves layout after split', async () => {
      render(() => <PaneContainer />);

      await waitFor(() => {
        expect((window as any).paneActions.getPaneCount()).toBe(1);
      });

      (window as any).paneActions.splitVertical();

      await waitFor(() => {
        const layout = JSON.parse(localStorage.getItem('agentmux.layout')!);
        expect(layout.panes).toHaveLength(2);
        expect(layout.orientation).toBe('vertical');
      });
    });

    it('restores layout from localStorage', async () => {
      // Pre-populate localStorage with saved layout
      const savedLayout = {
        panes: [
          { id: 'pane-1', agentInstanceName: 'Claude-1' },
          { id: 'pane-2', agentInstanceName: 'Claude-2' },
        ],
        orientation: 'horizontal',
        activePaneId: 'pane-2',
      };
      localStorage.setItem('agentmux.layout', JSON.stringify(savedLayout));

      // Mock existing agents
      mockInvoke.mockImplementation((command: string) => {
        if (command === 'list_claude_instances') {
          return Promise.resolve([
            { instanceName: 'Claude-1', pid: 11111, wsPort: 9001, status: 'running' },
            { instanceName: 'Claude-2', pid: 22222, wsPort: 9002, status: 'running' },
          ]);
        }
        return Promise.resolve();
      });

      const onPanesChange = vi.fn();
      render(() => <PaneContainer onPanesChange={onPanesChange} />);

      await waitFor(() => {
        expect((window as any).paneActions.getPaneCount()).toBe(2);
        expect(onPanesChange).toHaveBeenCalledWith(2);
      }, { timeout: 3000 });
    });

    it('spawns new agents for missing ones on restore', async () => {
      // Saved layout has 2 panes
      const savedLayout = {
        panes: [
          { id: 'pane-1', agentInstanceName: 'Claude-1' },
          { id: 'pane-2', agentInstanceName: null }, // No agent assigned
        ],
        orientation: 'vertical',
        activePaneId: 'pane-1',
      };
      localStorage.setItem('agentmux.layout', JSON.stringify(savedLayout));

      // Only Claude-1 exists
      mockInvoke.mockImplementation((command: string, args?: any) => {
        if (command === 'list_claude_instances') {
          return Promise.resolve([
            { instanceName: 'Claude-1', pid: 11111, wsPort: 9001, status: 'running' },
          ]);
        }
        if (command === 'spawn_embedded_claude') {
          return Promise.resolve({
            instanceName: args.instanceName,
            pid: 33333,
            wsPort: 9003,
            status: 'running',
          });
        }
        return Promise.resolve();
      });

      render(() => <PaneContainer />);

      await waitFor(() => {
        // Should spawn agent for pane-2
        expect(mockInvoke).toHaveBeenCalledWith('spawn_embedded_claude', expect.any(Object));
      }, { timeout: 3000 });
    });
  });

  it('reuses existing agents before spawning new ones', async () => {
    // Mock existing agent
    mockInvoke.mockImplementation((command: string) => {
      if (command === 'list_claude_instances') {
        return Promise.resolve([
          { instanceName: 'Existing-Agent', pid: 99999, wsPort: 9999, status: 'running' },
        ]);
      }
      return Promise.resolve();
    });

    render(() => <PaneContainer />);

    // Wait for agent to be assigned
    await waitFor(() => {
      const paneTitle = screen.getByText('Existing-Agent');
      expect(paneTitle).toBeTruthy();
    }, { timeout: 3000 });

    // Should NOT have called spawn_embedded_claude
    expect(mockInvoke).not.toHaveBeenCalledWith('spawn_embedded_claude', expect.any(Object));
  });

  it('shows loading state while spawning agent', async () => {
    mockInvoke.mockImplementation((command: string) => {
      if (command === 'list_claude_instances') {
        return Promise.resolve([]);
      }
      if (command === 'spawn_embedded_claude') {
        // Delay to show loading state
        return new Promise(resolve => setTimeout(() => resolve({
          instanceName: 'Claude-1',
          pid: 12345,
          wsPort: 9001,
          status: 'running',
        }), 1000));
      }
      return Promise.resolve();
    });

    render(() => <PaneContainer />);

    // Should show loading initially
    const loading = screen.getByText(/Starting agent\.\.\./i);
    expect(loading).toBeTruthy();

    // Wait for agent to load (check for pane title)
    await waitFor(() => {
      const paneTitle = screen.getByText('Claude-1');
      expect(paneTitle).toBeTruthy();
    }, { timeout: 3000 });
  });
});

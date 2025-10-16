import { Component, createSignal, For, Show, onMount, createEffect } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import EmbeddedTerminal from './EmbeddedTerminal';

export interface Pane {
  id: string;
  agent: Agent | null;
  isLoading: boolean;
}

interface Agent {
  instanceName: string;
  pid: number;
  wsPort: number;
  status: string;
}

export type SplitOrientation = 'vertical' | 'horizontal' | 'grid';

interface PaneContainerProps {
  onPanesChange?: (count: number) => void;
}

interface PersistedLayout {
  panes: Array<{ id: string; agentInstanceName: string | null }>;
  orientation: SplitOrientation;
  activePaneId: string;
}

// localStorage helpers
const STORAGE_KEY = 'agentmux.layout';

const saveLayout = (layout: PersistedLayout) => {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(layout));
    console.log('[PaneContainer] Layout saved to localStorage');
  } catch (err) {
    console.error('[PaneContainer] Failed to save layout:', err);
  }
};

const loadLayout = (): PersistedLayout | null => {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored) {
      const layout = JSON.parse(stored);
      console.log('[PaneContainer] Layout restored from localStorage:', layout);
      return layout;
    }
  } catch (err) {
    console.error('[PaneContainer] Failed to load layout:', err);
  }
  return null;
};

const PaneContainer: Component<PaneContainerProps> = (props) => {
  const [panes, setPanes] = createSignal<Pane[]>([
    { id: 'pane-1', agent: null, isLoading: true }
  ]);
  const [orientation, setOrientation] = createSignal<SplitOrientation>('vertical');
  const [activePaneId, setActivePaneId] = createSignal<string>('pane-1');
  const [isRestoring, setIsRestoring] = createSignal(false);

  // Auto-spawn agent for first pane
  const spawnAgentForPane = async (paneId: string) => {
    try {
      // Check for existing agents first
      const existingAgents: Agent[] = await invoke('list_claude_instances');

      const paneIndex = panes().findIndex(p => p.id === paneId);

      // Try to use an existing agent that's not already assigned
      const usedAgents = panes().map(p => p.agent?.instanceName).filter(Boolean);
      const availableAgent = existingAgents.find(a => !usedAgents.includes(a.instanceName));

      if (availableAgent) {
        setPanes(prev => prev.map(p =>
          p.id === paneId
            ? { ...p, agent: availableAgent, isLoading: false }
            : p
        ));
        console.log(`[PaneContainer] Assigned existing agent ${availableAgent.instanceName} to pane ${paneId}`);
      } else {
        // Spawn a new agent
        const instanceName = `Claude-${paneIndex + 1}`;
        const newAgent: Agent = await invoke('spawn_embedded_claude', {
          instanceName,
          workspacePath: '~'
        });

        setPanes(prev => prev.map(p =>
          p.id === paneId
            ? { ...p, agent: newAgent, isLoading: false }
            : p
        ));
        console.log(`[PaneContainer] Spawned new agent ${instanceName} for pane ${paneId}`);
      }
    } catch (err) {
      console.error(`[PaneContainer] Failed to spawn agent for pane ${paneId}:`, err);
      setPanes(prev => prev.map(p =>
        p.id === paneId
          ? { ...p, isLoading: false }
          : p
      ));
    }
  };

  // Restore layout from localStorage on mount
  onMount(async () => {
    const savedLayout = loadLayout();

    if (savedLayout && savedLayout.panes.length > 0) {
      setIsRestoring(true);
      console.log('[PaneContainer] Restoring layout...');

      // Get existing agents
      const existingAgents: Agent[] = await invoke('list_claude_instances').catch(() => []);

      // Restore panes
      const restoredPanes: Pane[] = savedLayout.panes.map(savedPane => {
        const agent = savedPane.agentInstanceName
          ? existingAgents.find(a => a.instanceName === savedPane.agentInstanceName) || null
          : null;

        return {
          id: savedPane.id,
          agent,
          isLoading: !agent // Will spawn if no agent found
        };
      });

      setPanes(restoredPanes);
      setOrientation(savedLayout.orientation);
      setActivePaneId(savedLayout.activePaneId);
      props.onPanesChange?.(restoredPanes.length);

      // Spawn agents for panes that need them
      for (const pane of restoredPanes) {
        if (!pane.agent) {
          spawnAgentForPane(pane.id);
        }
      }

      setIsRestoring(false);
    } else {
      // No saved layout, initialize first pane
      setTimeout(() => spawnAgentForPane('pane-1'), 100);
    }
  });

  // Save layout to localStorage whenever it changes
  createEffect(() => {
    if (isRestoring()) return; // Don't save during restoration

    const currentPanes = panes();
    const currentOrientation = orientation();
    const currentActivePaneId = activePaneId();

    const layout: PersistedLayout = {
      panes: currentPanes.map(p => ({
        id: p.id,
        agentInstanceName: p.agent?.instanceName || null
      })),
      orientation: currentOrientation,
      activePaneId: currentActivePaneId
    };

    saveLayout(layout);
  });

  const splitVertical = () => {
    const newPaneId = `pane-${Date.now()}`;
    setPanes(prev => {
      const newPanes = [...prev, { id: newPaneId, agent: null, isLoading: true }];
      props.onPanesChange?.(newPanes.length);
      return newPanes;
    });
    setOrientation('vertical');
    spawnAgentForPane(newPaneId);
  };

  const splitHorizontal = () => {
    const newPaneId = `pane-${Date.now()}`;
    setPanes(prev => {
      const newPanes = [...prev, { id: newPaneId, agent: null, isLoading: true }];
      props.onPanesChange?.(newPanes.length);
      return newPanes;
    });
    setOrientation('horizontal');
    spawnAgentForPane(newPaneId);
  };

  const closePane = (paneId: string) => {
    if (panes().length === 1) {
      console.warn('[PaneContainer] Cannot close the last pane');
      return;
    }

    setPanes(prev => {
      const newPanes = prev.filter(p => p.id !== paneId);
      props.onPanesChange?.(newPanes.length);
      return newPanes;
    });

    // If we closed the active pane, activate the first remaining pane
    if (activePaneId() === paneId) {
      const remaining = panes().filter(p => p.id !== paneId);
      if (remaining.length > 0) {
        setActivePaneId(remaining[0].id);
      }
    }
  };

  const resetToSingle = () => {
    const firstPane = panes()[0];
    setPanes([firstPane]);
    setActivePaneId(firstPane.id);
    setOrientation('vertical');
    props.onPanesChange?.(1);
  };

  // Expose methods to parent via window object (for menu actions)
  (window as any).paneActions = {
    splitVertical,
    splitHorizontal,
    closeCurrentPane: () => closePane(activePaneId()),
    resetToSingle,
    getPaneCount: () => panes().length
  };

  const getContainerClass = () => {
    const count = panes().length;
    if (count === 1) return 'pane-container-single';
    if (count === 2) {
      return orientation() === 'vertical'
        ? 'pane-container-vertical'
        : 'pane-container-horizontal';
    }
    return 'pane-container-grid';
  };

  return (
    <div class={`pane-container ${getContainerClass()}`}>
      <For each={panes()}>
        {(pane) => (
          <div
            class={`pane ${activePaneId() === pane.id ? 'pane-active' : ''}`}
            onClick={() => setActivePaneId(pane.id)}
          >
            <div class="pane-header">
              <span class="pane-title">
                <span class={`status-dot ${pane.agent ? 'online' : 'offline'}`}></span>
                {pane.agent?.instanceName || 'Loading...'}
              </span>
              <Show when={panes().length > 1}>
                <button
                  class="pane-close-btn"
                  onClick={(e) => {
                    e.stopPropagation();
                    closePane(pane.id);
                  }}
                  title="Close pane"
                >
                  ✕
                </button>
              </Show>
            </div>

            <div class="pane-content">
              <Show when={pane.isLoading}>
                <div class="pane-loading">
                  <div class="loading-spinner"></div>
                  <p>Starting agent...</p>
                </div>
              </Show>

              <Show when={pane.agent && !pane.isLoading && pane.agent.wsPort}>
                {(currentAgent) => (
                  <EmbeddedTerminal
                    instanceName={currentAgent().instanceName}
                    wsPort={currentAgent().wsPort}
                  />
                )}
              </Show>

              <Show when={pane.agent && !pane.isLoading && !pane.agent.wsPort}>
                <div class="pane-error">
                  <p>⚠️ Agent running but WebSocket port unavailable</p>
                  <p class="error-detail">Agent: {pane.agent?.instanceName}</p>
                </div>
              </Show>
            </div>
          </div>
        )}
      </For>
    </div>
  );
};

export default PaneContainer;

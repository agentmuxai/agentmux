import { Component, createSignal, For, Show } from 'solid-js';
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

const PaneContainer: Component<PaneContainerProps> = (props) => {
  const [panes, setPanes] = createSignal<Pane[]>([
    { id: 'pane-1', agent: null, isLoading: true }
  ]);
  const [orientation, setOrientation] = createSignal<SplitOrientation>('vertical');
  const [activePaneId, setActivePaneId] = createSignal<string>('pane-1');

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

  // Initialize first pane
  setTimeout(() => spawnAgentForPane('pane-1'), 100);

  const splitVertical = () => {
    const newPaneId = `pane-${Date.now()}`;
    setPanes(prev => [...prev, { id: newPaneId, agent: null, isLoading: true }]);
    setOrientation('vertical');
    spawnAgentForPane(newPaneId);
    props.onPanesChange?.(panes().length + 1);
  };

  const splitHorizontal = () => {
    const newPaneId = `pane-${Date.now()}`;
    setPanes(prev => [...prev, { id: newPaneId, agent: null, isLoading: true }]);
    setOrientation('horizontal');
    spawnAgentForPane(newPaneId);
    props.onPanesChange?.(panes().length + 1);
  };

  const closePane = (paneId: string) => {
    if (panes().length === 1) {
      console.warn('[PaneContainer] Cannot close the last pane');
      return;
    }

    setPanes(prev => prev.filter(p => p.id !== paneId));

    // If we closed the active pane, activate the first remaining pane
    if (activePaneId() === paneId) {
      const remaining = panes().filter(p => p.id !== paneId);
      if (remaining.length > 0) {
        setActivePaneId(remaining[0].id);
      }
    }

    props.onPanesChange?.(panes().length - 1);
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

              <Show when={pane.agent && !pane.isLoading}>
                {(currentAgent) => (
                  <EmbeddedTerminal
                    instanceName={currentAgent().instanceName}
                    wsPort={currentAgent().wsPort}
                  />
                )}
              </Show>
            </div>
          </div>
        )}
      </For>
    </div>
  );
};

export default PaneContainer;

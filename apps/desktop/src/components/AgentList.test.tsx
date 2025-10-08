import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, waitFor } from '@solidjs/testing-library';
import AgentList from './AgentList';
import { invoke } from '@tauri-apps/api/core';

// Mock Tauri invoke
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}));

describe('AgentList Component', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.clearAllTimers();
  });

  it('renders the component with no agents', async () => {
    (invoke as any).mockResolvedValue([]);

    render(() => <AgentList />);

    await waitFor(() => {
      expect(screen.getByText('Agent Registry (0 total)')).toBeInTheDocument();
    });

    expect(
      screen.getByText(/No agents connected. Start the bus and connect agents/)
    ).toBeInTheDocument();
  });

  it('displays agents when they are connected', async () => {
    const mockAgents = [
      {
        id: 'agent-1',
        name: 'TestAgent',
        workspace: '/workspace',
        status: 'online',
        connected_at: 1234567890,
        uptime: 3600, // 1 hour
        messages_sent: 10,
        messages_received: 5,
      },
    ];

    (invoke as any).mockResolvedValue(mockAgents);

    render(() => <AgentList />);

    await waitFor(() => {
      expect(screen.getByText('Agent Registry (1 total)')).toBeInTheDocument();
    });

    expect(screen.getByText(/TestAgent \(agent-1\)/)).toBeInTheDocument();
    expect(screen.getByText(/\/workspace/)).toBeInTheDocument();
    expect(screen.getByText(/↑ 10 ↓ 5/)).toBeInTheDocument();
  });

  it('formats uptime correctly for hours and minutes', async () => {
    const mockAgents = [
      {
        id: 'agent-1',
        name: 'Agent1',
        workspace: '/test',
        status: 'online',
        connected_at: 0,
        uptime: 7320, // 2 hours 2 minutes
        messages_sent: 0,
        messages_received: 0,
      },
    ];

    (invoke as any).mockResolvedValue(mockAgents);

    render(() => <AgentList />);

    await waitFor(() => {
      expect(screen.getByText(/Uptime: 2h 2m/)).toBeInTheDocument();
    });
  });

  it('formats uptime correctly for minutes only', async () => {
    const mockAgents = [
      {
        id: 'agent-2',
        name: 'Agent2',
        workspace: '/test',
        status: 'online',
        connected_at: 0,
        uptime: 600, // 10 minutes
        messages_sent: 0,
        messages_received: 0,
      },
    ];

    (invoke as any).mockResolvedValue(mockAgents);

    render(() => <AgentList />);

    await waitFor(() => {
      expect(screen.getByText(/Uptime: 10m/)).toBeInTheDocument();
    });
  });

  it('displays multiple agents', async () => {
    const mockAgents = [
      {
        id: 'agent-1',
        name: 'Agent1',
        workspace: '/workspace1',
        status: 'online',
        connected_at: 0,
        uptime: 100,
        messages_sent: 5,
        messages_received: 3,
      },
      {
        id: 'agent-2',
        name: 'Agent2',
        workspace: '/workspace2',
        status: 'busy',
        connected_at: 0,
        uptime: 200,
        messages_sent: 15,
        messages_received: 10,
      },
    ];

    (invoke as any).mockResolvedValue(mockAgents);

    render(() => <AgentList />);

    await waitFor(() => {
      expect(screen.getByText('Agent Registry (2 total)')).toBeInTheDocument();
    });

    expect(screen.getByText(/Agent1 \(agent-1\)/)).toBeInTheDocument();
    expect(screen.getByText(/Agent2 \(agent-2\)/)).toBeInTheDocument();
  });

  it('renders disconnect button for each agent', async () => {
    const mockAgents = [
      {
        id: 'agent-1',
        name: 'Agent1',
        workspace: '/test',
        status: 'online',
        connected_at: 0,
        uptime: 100,
        messages_sent: 0,
        messages_received: 0,
      },
    ];

    (invoke as any).mockResolvedValue(mockAgents);

    render(() => <AgentList />);

    await waitFor(() => {
      const disconnectButtons = screen.getAllByText('Disconnect');
      expect(disconnectButtons).toHaveLength(1);
    });
  });

  it('displays status dot for agents', async () => {
    const mockAgents = [
      {
        id: 'agent-1',
        name: 'Agent1',
        workspace: '/test',
        status: 'online',
        connected_at: 0,
        uptime: 100,
        messages_sent: 0,
        messages_received: 0,
      },
    ];

    (invoke as any).mockResolvedValue(mockAgents);

    render(() => <AgentList />);

    await waitFor(() => {
      const statusDots = document.querySelectorAll('.status-dot.online');
      expect(statusDots).toHaveLength(1);
    });
  });

  it('calls invoke to fetch agents on mount', async () => {
    (invoke as any).mockResolvedValue([]);

    render(() => <AgentList />);

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith('get_connected_agents');
    });
  });

  it('handles fetch errors gracefully', async () => {
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {});
    (invoke as any).mockRejectedValue(new Error('Network error'));

    render(() => <AgentList />);

    await waitFor(() => {
      expect(consoleError).toHaveBeenCalledWith(
        'Failed to get agents:',
        expect.any(Error)
      );
    });

    consoleError.mockRestore();
  });

  it('renders Connection Statistics section', async () => {
    (invoke as any).mockResolvedValue([]);

    render(() => <AgentList />);

    await waitFor(() => {
      expect(screen.getByText('Connection Statistics')).toBeInTheDocument();
    });

    expect(
      screen.getByText(/Detailed statistics will be shown here/)
    ).toBeInTheDocument();
  });

  it('displays correct message count format', async () => {
    const mockAgents = [
      {
        id: 'agent-1',
        name: 'Agent1',
        workspace: '/test',
        status: 'online',
        connected_at: 0,
        uptime: 100,
        messages_sent: 42,
        messages_received: 33,
      },
    ];

    (invoke as any).mockResolvedValue(mockAgents);

    render(() => <AgentList />);

    await waitFor(() => {
      expect(screen.getByText(/↑ 42 ↓ 33/)).toBeInTheDocument();
    });
  });

  it('updates agent count in header', async () => {
    (invoke as any).mockResolvedValue([
      {
        id: 'agent-1',
        name: 'Agent1',
        workspace: '/test',
        status: 'online',
        connected_at: 0,
        uptime: 100,
        messages_sent: 0,
        messages_received: 0,
      },
      {
        id: 'agent-2',
        name: 'Agent2',
        workspace: '/test',
        status: 'online',
        connected_at: 0,
        uptime: 100,
        messages_sent: 0,
        messages_received: 0,
      },
      {
        id: 'agent-3',
        name: 'Agent3',
        workspace: '/test',
        status: 'online',
        connected_at: 0,
        uptime: 100,
        messages_sent: 0,
        messages_received: 0,
      },
    ]);

    render(() => <AgentList />);

    await waitFor(() => {
      expect(screen.getByText('Agent Registry (3 total)')).toBeInTheDocument();
    });
  });
});

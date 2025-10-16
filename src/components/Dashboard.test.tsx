import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@solidjs/testing-library';
import Dashboard from './Dashboard';
import { invoke } from '@tauri-apps/api/core';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}));

describe('Dashboard Component', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('renders the dashboard with initial state', async () => {
    (invoke as any).mockResolvedValue({
      running: false,
      agents_connected: 0,
      messages_per_second: 0,
      total_messages: 0,
    });

    render(() => <Dashboard />);

    await waitFor(() => {
      expect(screen.getByText('Server Bus Control')).toBeInTheDocument();
    });

    expect(screen.getByText(/Status: Stopped/)).toBeInTheDocument();
  });

  it('displays running status when bus is active', async () => {
    (invoke as any).mockResolvedValue({
      running: true,
      agents_connected: 3,
      messages_per_second: 15,
      total_messages: 1000,
    });

    render(() => <Dashboard />);

    await waitFor(() => {
      expect(screen.getByText(/Status: Running/)).toBeInTheDocument();
    });
  });

  it('displays connected agents count', async () => {
    (invoke as any).mockResolvedValue({
      running: true,
      agents_connected: 5,
      messages_per_second: 0,
      total_messages: 0,
    });

    render(() => <Dashboard />);

    await waitFor(() => {
      expect(screen.getByText('5')).toBeInTheDocument();
      expect(screen.getByText('Connected Agents')).toBeInTheDocument();
    });
  });

  it('displays messages per second and total messages', async () => {
    (invoke as any).mockResolvedValue({
      running: true,
      agents_connected: 2,
      messages_per_second: 42,
      total_messages: 1500,
    });

    render(() => <Dashboard />);

    await waitFor(() => {
      expect(screen.getByText('42')).toBeInTheDocument();
      expect(screen.getByText('1500 total')).toBeInTheDocument();
    });
  });

  it('disables start button when bus is running', async () => {
    (invoke as any).mockResolvedValue({
      running: true,
      agents_connected: 0,
      messages_per_second: 0,
      total_messages: 0,
    });

    render(() => <Dashboard />);

    await waitFor(() => {
      const startButton = screen.getByRole('button', { name: /Start Bus/ });
      expect(startButton).toBeDisabled();
    });
  });

  it('disables stop button when bus is not running', async () => {
    (invoke as any).mockResolvedValue({
      running: false,
      agents_connected: 0,
      messages_per_second: 0,
      total_messages: 0,
    });

    render(() => <Dashboard />);

    await waitFor(() => {
      const stopButton = screen.getByRole('button', { name: /Stop Bus/ });
      expect(stopButton).toBeDisabled();
    });
  });

  it('calls start_bus when start button is clicked', async () => {
    (invoke as any)
      .mockResolvedValueOnce({
        running: false,
        agents_connected: 0,
        messages_per_second: 0,
        total_messages: 0,
      })
      .mockResolvedValueOnce('Bus started')
      .mockResolvedValueOnce({
        running: true,
        agents_connected: 0,
        messages_per_second: 0,
        total_messages: 0,
      });

    render(() => <Dashboard />);

    await waitFor(() => {
      expect(screen.getByText(/Status: Stopped/)).toBeInTheDocument();
    });

    const startButton = screen.getByRole('button', { name: /Start Bus/ });
    fireEvent.click(startButton);

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith('start_bus', {
        config: {
          host: 'localhost',
          port: 8765,
          max_agents: 50,
        },
      });
    });
  });

  it('calls stop_bus when stop button is clicked', async () => {
    (invoke as any)
      .mockResolvedValueOnce({
        running: true,
        agents_connected: 0,
        messages_per_second: 0,
        total_messages: 0,
      })
      .mockResolvedValueOnce('Bus stopped')
      .mockResolvedValueOnce({
        running: false,
        agents_connected: 0,
        messages_per_second: 0,
        total_messages: 0,
      });

    render(() => <Dashboard />);

    await waitFor(() => {
      expect(screen.getByText(/Status: Running/)).toBeInTheDocument();
    });

    const stopButton = screen.getByRole('button', { name: /Stop Bus/ });
    fireEvent.click(stopButton);

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith('stop_bus');
    });
  });

  it('displays error message when start fails', async () => {
    (invoke as any)
      .mockResolvedValueOnce({
        running: false,
        agents_connected: 0,
        messages_per_second: 0,
        total_messages: 0,
      })
      .mockRejectedValueOnce(new Error('Port already in use'));

    render(() => <Dashboard />);

    await waitFor(() => {
      expect(screen.getByText(/Status: Stopped/)).toBeInTheDocument();
    });

    const startButton = screen.getByRole('button', { name: /Start Bus/ });
    fireEvent.click(startButton);

    await waitFor(() => {
      expect(screen.getByText(/Port already in use/)).toBeInTheDocument();
    });
  });

  it('displays error message when stop fails', async () => {
    (invoke as any)
      .mockResolvedValueOnce({
        running: true,
        agents_connected: 0,
        messages_per_second: 0,
        total_messages: 0,
      })
      .mockRejectedValueOnce(new Error('Bus is not running'));

    render(() => <Dashboard />);

    await waitFor(() => {
      expect(screen.getByText(/Status: Running/)).toBeInTheDocument();
    });

    const stopButton = screen.getByRole('button', { name: /Stop Bus/ });
    fireEvent.click(stopButton);

    await waitFor(() => {
      expect(screen.getByText(/Bus is not running/)).toBeInTheDocument();
    });
  });

  it('clears error when successful operation occurs', async () => {
    (invoke as any)
      .mockResolvedValueOnce({
        running: false,
        agents_connected: 0,
        messages_per_second: 0,
        total_messages: 0,
      })
      .mockRejectedValueOnce(new Error('Error 1'))
      .mockResolvedValueOnce('Bus started')
      .mockResolvedValueOnce({
        running: true,
        agents_connected: 0,
        messages_per_second: 0,
        total_messages: 0,
      });

    render(() => <Dashboard />);

    await waitFor(() => {
      expect(screen.getByText(/Status: Stopped/)).toBeInTheDocument();
    });

    const startButton = screen.getByRole('button', { name: /Start Bus/ });

    // First click fails
    fireEvent.click(startButton);
    await waitFor(() => {
      expect(screen.getByText(/Error 1/)).toBeInTheDocument();
    });

    // Second click succeeds
    fireEvent.click(startButton);
    await waitFor(() => {
      expect(screen.queryByText(/Error:/)).not.toBeInTheDocument();
    });
  });

  it('shows Live status for connected agents when bus is running', async () => {
    (invoke as any).mockResolvedValue({
      running: true,
      agents_connected: 3,
      messages_per_second: 0,
      total_messages: 0,
    });

    render(() => <Dashboard />);

    await waitFor(() => {
      expect(screen.getByText('Live')).toBeInTheDocument();
    });
  });

  it('shows Offline status when bus is stopped', async () => {
    (invoke as any).mockResolvedValue({
      running: false,
      agents_connected: 0,
      messages_per_second: 0,
      total_messages: 0,
    });

    render(() => <Dashboard />);

    await waitFor(() => {
      expect(screen.getByText('Offline')).toBeInTheDocument();
    });
  });

  it('displays WebSocket URL in bus status', async () => {
    (invoke as any).mockResolvedValue({
      running: false,
      agents_connected: 0,
      messages_per_second: 0,
      total_messages: 0,
    });

    render(() => <Dashboard />);

    await waitFor(() => {
      expect(screen.getByText('ws://localhost:8765')).toBeInTheDocument();
    });
  });

  it('shows different recent activity messages based on bus state', async () => {
    (invoke as any).mockResolvedValue({
      running: false,
      agents_connected: 0,
      messages_per_second: 0,
      total_messages: 0,
    });

    render(() => <Dashboard />);

    await waitFor(() => {
      expect(
        screen.getByText(/Start the bus to begin monitoring agents/)
      ).toBeInTheDocument();
    });
  });

  it('polls for status updates every 2 seconds', async () => {
    vi.useFakeTimers();
    (invoke as any).mockResolvedValue({
      running: false,
      agents_connected: 0,
      messages_per_second: 0,
      total_messages: 0,
    });

    render(() => <Dashboard />);

    // Initial call
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith('get_bus_status');
    });

    const initialCallCount = (invoke as any).mock.calls.length;

    // Advance time by 2 seconds
    await vi.advanceTimersByTimeAsync(2000);

    await waitFor(() => {
      expect((invoke as any).mock.calls.length).toBeGreaterThan(initialCallCount);
    });

    vi.useRealTimers();
  });

  it('displays checkmark when bus is running', async () => {
    (invoke as any).mockResolvedValue({
      running: true,
      agents_connected: 0,
      messages_per_second: 0,
      total_messages: 0,
    });

    render(() => <Dashboard />);

    await waitFor(() => {
      expect(screen.getByText('✓')).toBeInTheDocument();
    });
  });

  it('displays X when bus is stopped', async () => {
    (invoke as any).mockResolvedValue({
      running: false,
      agents_connected: 0,
      messages_per_second: 0,
      total_messages: 0,
    });

    render(() => <Dashboard />);

    await waitFor(() => {
      expect(screen.getByText('✗')).toBeInTheDocument();
    });
  });

  it('handles status update errors gracefully', async () => {
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {});
    (invoke as any).mockRejectedValue(new Error('Network error'));

    render(() => <Dashboard />);

    await waitFor(() => {
      expect(consoleError).toHaveBeenCalledWith(
        'Failed to get bus status:',
        expect.any(Error)
      );
    });

    consoleError.mockRestore();
  });
});

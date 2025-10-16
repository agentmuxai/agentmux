import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@solidjs/testing-library';
import BusControl from './BusControl';

describe('BusControl Component', () => {
  it('renders the component with default values', () => {
    render(() => <BusControl />);

    expect(screen.getByText('Bus Configuration')).toBeInTheDocument();
    expect(screen.getByText('Connection Info')).toBeInTheDocument();
    expect(screen.getByText('Performance Metrics')).toBeInTheDocument();
  });

  it('displays default host, port, and max agents', () => {
    render(() => <BusControl />);

    const hostInput = screen.getByDisplayValue('localhost');
    const portInput = screen.getByDisplayValue('8765');
    const maxAgentsInput = screen.getByDisplayValue('50');

    expect(hostInput).toBeInTheDocument();
    expect(portInput).toBeInTheDocument();
    expect(maxAgentsInput).toBeInTheDocument();
  });

  it('updates host input value', () => {
    render(() => <BusControl />);

    const hostInput = screen.getByDisplayValue('localhost') as HTMLInputElement;
    fireEvent.input(hostInput, { target: { value: '127.0.0.1' } });

    expect(hostInput.value).toBe('127.0.0.1');
  });

  it('updates port input value', () => {
    render(() => <BusControl />);

    const portInput = screen.getByDisplayValue('8765') as HTMLInputElement;
    fireEvent.input(portInput, { target: { value: '9000' } });

    expect(portInput.value).toBe('9000');
  });

  it('updates max agents input value', () => {
    render(() => <BusControl />);

    const maxAgentsInput = screen.getByDisplayValue('50') as HTMLInputElement;
    fireEvent.input(maxAgentsInput, { target: { value: '100' } });

    expect(maxAgentsInput.value).toBe('100');
  });

  it('displays connection URLs correctly with default values', () => {
    render(() => <BusControl />);

    expect(screen.getByText(/ws:\/\/localhost:8765/)).toBeInTheDocument();
    expect(screen.getByText(/http:\/\/localhost:8765\/health/)).toBeInTheDocument();
    expect(screen.getByText(/http:\/\/localhost:8765\/metrics/)).toBeInTheDocument();
  });

  it('updates connection URLs when host changes', () => {
    render(() => <BusControl />);

    const hostInput = screen.getByDisplayValue('localhost') as HTMLInputElement;
    fireEvent.input(hostInput, { target: { value: '192.168.1.100' } });

    expect(screen.getByText(/ws:\/\/192.168.1.100:8765/)).toBeInTheDocument();
    expect(screen.getByText(/http:\/\/192.168.1.100:8765\/health/)).toBeInTheDocument();
  });

  it('updates connection URLs when port changes', () => {
    render(() => <BusControl />);

    const portInput = screen.getByDisplayValue('8765') as HTMLInputElement;
    fireEvent.input(portInput, { target: { value: '3000' } });

    expect(screen.getByText(/ws:\/\/localhost:3000/)).toBeInTheDocument();
    expect(screen.getByText(/http:\/\/localhost:3000\/metrics/)).toBeInTheDocument();
  });

  it('renders Save Config button', () => {
    render(() => <BusControl />);

    const saveButton = screen.getByText(/💾 Save Config/);
    expect(saveButton).toBeInTheDocument();
    expect(saveButton.tagName).toBe('BUTTON');
  });

  it('renders Restart Bus button', () => {
    render(() => <BusControl />);

    const restartButton = screen.getByText(/🔄 Restart Bus/);
    expect(restartButton).toBeInTheDocument();
    expect(restartButton.tagName).toBe('BUTTON');
  });

  it('renders WebSocket protocol selector', () => {
    render(() => <BusControl />);

    const select = screen.getByRole('combobox');
    expect(select).toBeInTheDocument();
    expect(screen.getByText('WebSocket')).toBeInTheDocument();
  });

  it('shows performance metrics placeholder', () => {
    render(() => <BusControl />);

    expect(
      screen.getByText(/Charts will be displayed here once the bus is running/)
    ).toBeInTheDocument();
  });

  it('displays all input labels', () => {
    render(() => <BusControl />);

    expect(screen.getByText('Protocol:')).toBeInTheDocument();
    expect(screen.getByText('Host:')).toBeInTheDocument();
    expect(screen.getByText('Port:')).toBeInTheDocument();
    expect(screen.getByText('Max Agents:')).toBeInTheDocument();
  });
});

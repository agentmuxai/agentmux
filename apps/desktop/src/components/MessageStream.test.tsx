import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@solidjs/testing-library';
import MessageStream from './MessageStream';

describe('MessageStream Component', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders the component with empty state', () => {
    render(() => <MessageStream />);

    expect(screen.getByText('Message Stream')).toBeInTheDocument();
    expect(screen.getByText(/No messages yet/)).toBeInTheDocument();
  });

  it('displays message count', () => {
    render(() => <MessageStream />);

    expect(screen.getByText(/0 \/ 0 messages/)).toBeInTheDocument();
  });

  it('renders pause button', () => {
    render(() => <MessageStream />);

    const pauseButton = screen.getByRole('button', { name: /Pause/ });
    expect(pauseButton).toBeInTheDocument();
  });

  it('renders clear button', () => {
    render(() => <MessageStream />);

    const clearButton = screen.getByRole('button', { name: /Clear/ });
    expect(clearButton).toBeInTheDocument();
  });

  it('toggles pause state when pause button is clicked', () => {
    render(() => <MessageStream />);

    const pauseButton = screen.getByRole('button', { name: /Pause/ });
    fireEvent.click(pauseButton);

    expect(screen.getByRole('button', { name: /Resume/ })).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: /Resume/ }));
    expect(screen.getByRole('button', { name: /Pause/ })).toBeInTheDocument();
  });

  it('renders filter input', () => {
    render(() => <MessageStream />);

    const filterInput = screen.getByPlaceholderText(/Filter messages/);
    expect(filterInput).toBeInTheDocument();
  });

  it('updates filter value on input', () => {
    render(() => <MessageStream />);

    const filterInput = screen.getByPlaceholderText(/Filter messages/) as HTMLInputElement;
    fireEvent.input(filterInput, { target: { value: 'test' } });

    expect(filterInput.value).toBe('test');
  });

  it('renders max messages selector', () => {
    render(() => <MessageStream />);

    const selector = screen.getByRole('combobox');
    expect(selector).toBeInTheDocument();
    expect(screen.getByText('Last 100')).toBeInTheDocument();
  });

  it('updates max messages when selector changes', () => {
    render(() => <MessageStream />);

    const selector = screen.getByRole('combobox') as HTMLSelectElement;
    fireEvent.change(selector, { target: { value: '250' } });

    expect(selector.value).toBe('250');
  });

  it('displays message statistics section', () => {
    render(() => <MessageStream />);

    expect(screen.getByText('Message Statistics')).toBeInTheDocument();
    expect(screen.getByText('Total Messages')).toBeInTheDocument();
    expect(screen.getByText('Broadcasts')).toBeInTheDocument();
    expect(screen.getByText('Direct Messages')).toBeInTheDocument();
    expect(screen.getByText('Stream Status')).toBeInTheDocument();
  });

  it('shows Live status by default', () => {
    render(() => <MessageStream />);

    expect(screen.getByText('Live')).toBeInTheDocument();
  });

  it('shows Paused status when paused', () => {
    render(() => <MessageStream />);

    const pauseButton = screen.getByRole('button', { name: /Pause/ });
    fireEvent.click(pauseButton);

    expect(screen.getByText('Paused')).toBeInTheDocument();
  });

  it('displays initial statistics with zero values', () => {
    render(() => <MessageStream />);

    const stats = screen.getAllByText('0');
    expect(stats.length).toBeGreaterThanOrEqual(3); // Total, Broadcasts, Direct
  });

  it('renders empty state message correctly', () => {
    render(() => <MessageStream />);

    expect(
      screen.getByText(/No messages yet. Messages will appear here when agents communicate./)
    ).toBeInTheDocument();
  });

  it('shows filter no results message when filter has no matches', async () => {
    render(() => <MessageStream />);

    const filterInput = screen.getByPlaceholderText(/Filter messages/);
    fireEvent.input(filterInput, { target: { value: 'nonexistent' } });

    // Should still show empty state since there are no messages
    expect(
      screen.getByText(/No messages yet/)
    ).toBeInTheDocument();
  });

  it('displays all selector options', () => {
    render(() => <MessageStream />);

    expect(screen.getByText('Last 50')).toBeInTheDocument();
    expect(screen.getByText('Last 100')).toBeInTheDocument();
    expect(screen.getByText('Last 250')).toBeInTheDocument();
    expect(screen.getByText('Last 500')).toBeInTheDocument();
  });

  it('has correct default max messages value', () => {
    render(() => <MessageStream />);

    const selector = screen.getByRole('combobox') as HTMLSelectElement;
    expect(selector.value).toBe('100');
  });

  it('renders message stream container', () => {
    render(() => <MessageStream />);

    const streamContainer = screen.getByText(/No messages yet/).parentElement;
    expect(streamContainer).toHaveStyle({ 'max-height': '600px' });
  });
});

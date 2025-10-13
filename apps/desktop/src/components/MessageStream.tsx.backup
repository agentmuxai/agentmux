import { Component, createSignal, onMount, onCleanup, For } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';

interface BusMessage {
  id: string;
  from: {
    id: string;
    name: string;
  };
  to: string;
  msg_type: string;
  payload: any;
  timestamp: number;
}

const MessageStream: Component = () => {
  const [messages, setMessages] = createSignal<BusMessage[]>([]);
  const [paused, setPaused] = createSignal(false);
  const [filter, setFilter] = createSignal('');
  const [maxMessages, setMaxMessages] = createSignal(100);

  const formatTimestamp = (timestamp: number): string => {
    const date = new Date(timestamp * 1000);
    return date.toLocaleTimeString('en-US', {
      hour12: false,
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit'
    });
  };

  const formatPayload = (payload: any): string => {
    try {
      return JSON.stringify(payload, null, 2);
    } catch {
      return String(payload);
    }
  };

  const addMessage = (message: BusMessage) => {
    if (!paused()) {
      setMessages(prev => {
        const updated = [message, ...prev];
        return updated.slice(0, maxMessages());
      });
    }
  };

  const clearMessages = () => {
    setMessages([]);
  };

  const filteredMessages = () => {
    const filterText = filter().toLowerCase();
    if (!filterText) return messages();

    return messages().filter(msg =>
      msg.from.id.toLowerCase().includes(filterText) ||
      msg.from.name.toLowerCase().includes(filterText) ||
      msg.to.toLowerCase().includes(filterText) ||
      msg.msg_type.toLowerCase().includes(filterText) ||
      JSON.stringify(msg.payload).toLowerCase().includes(filterText)
    );
  };

  const fetchMessages = async () => {
    try {
      const msgs = await invoke<BusMessage[]>('get_recent_messages', {
        limit: maxMessages()
      });
      setMessages(msgs);
    } catch (err) {
      console.error('Failed to fetch messages:', err);
    }
  };

  // Poll for messages every 2 seconds
  let intervalId: number;

  onMount(() => {
    fetchMessages(); // Initial fetch
    intervalId = window.setInterval(fetchMessages, 2000);
  });

  onCleanup(() => {
    if (intervalId) {
      clearInterval(intervalId);
    }
  });

  return (
    <div>
      <div class="card">
        <div style={{ display: 'flex', 'justify-content': 'space-between', 'align-items': 'center', 'margin-bottom': '1rem' }}>
          <h2>Message Stream</h2>
          <div style={{ display: 'flex', gap: '0.5rem', 'align-items': 'center' }}>
            <span style={{ color: '#999', 'font-size': '0.9rem' }}>
              {filteredMessages().length} / {messages().length} messages
            </span>
            <button
              class="secondary"
              onClick={() => setPaused(!paused())}
              style={{ padding: '0.5rem 1rem', 'font-size': '0.85rem' }}
            >
              {paused() ? '▶️ Resume' : '⏸️ Pause'}
            </button>
            <button
              class="danger"
              onClick={clearMessages}
              style={{ padding: '0.5rem 1rem', 'font-size': '0.85rem' }}
            >
              🗑️ Clear
            </button>
          </div>
        </div>

        <div style={{ 'margin-bottom': '1rem', display: 'flex', gap: '1rem', 'align-items': 'center' }}>
          <input
            type="text"
            placeholder="Filter messages (sender, type, payload...)"
            value={filter()}
            onInput={(e) => setFilter(e.currentTarget.value)}
            style={{ flex: 1 }}
          />
          <select
            value={maxMessages()}
            onChange={(e) => setMaxMessages(parseInt(e.currentTarget.value))}
            style={{ background: '#1a1a1a', border: '1px solid #3a3a3a', color: '#e0e0e0', padding: '0.5rem', 'border-radius': '4px' }}
          >
            <option value="50">Last 50</option>
            <option value="100">Last 100</option>
            <option value="250">Last 250</option>
            <option value="500">Last 500</option>
          </select>
        </div>

        <div
          class="message-stream"
          style={{
            'max-height': '600px',
            'overflow-y': 'auto',
            background: '#0a0a0a',
            border: '1px solid #2a2a2a',
            'border-radius': '8px',
            padding: '1rem'
          }}
        >
          {filteredMessages().length === 0 ? (
            <div style={{
              color: '#666',
              'text-align': 'center',
              padding: '3rem',
              'font-size': '0.95rem'
            }}>
              {messages().length === 0
                ? '📭 No messages yet. Messages will appear here when agents communicate.'
                : '🔍 No messages match your filter.'}
            </div>
          ) : (
            <For each={filteredMessages()}>
              {(message, index) => (
                <div
                  class="message-item"
                  style={{
                    background: '#1a1a1a',
                    border: '1px solid #2a2a2a',
                    'border-radius': '6px',
                    padding: '1rem',
                    'margin-bottom': '0.75rem',
                    'font-family': 'monospace',
                    'font-size': '0.85rem'
                  }}
                >
                  <div style={{
                    display: 'flex',
                    'justify-content': 'space-between',
                    'margin-bottom': '0.5rem',
                    'border-bottom': '1px solid #2a2a2a',
                    'padding-bottom': '0.5rem'
                  }}>
                    <div style={{ display: 'flex', gap: '1rem', 'align-items': 'center' }}>
                      <span style={{ color: '#4a9eff', 'font-weight': 'bold' }}>
                        {message.from.name}
                      </span>
                      <span style={{ color: '#666' }}>→</span>
                      <span style={{ color: message.to === '*' ? '#ff9800' : '#66bb6a' }}>
                        {message.to === '*' ? 'BROADCAST' : message.to}
                      </span>
                      <span
                        class="message-type-badge"
                        style={{
                          background: '#2a2a4a',
                          color: '#9c9cff',
                          padding: '0.25rem 0.5rem',
                          'border-radius': '4px',
                          'font-size': '0.75rem',
                          'font-weight': 'bold'
                        }}
                      >
                        {message.msg_type}
                      </span>
                    </div>
                    <span style={{ color: '#999', 'font-size': '0.8rem' }}>
                      {formatTimestamp(message.timestamp)}
                    </span>
                  </div>

                  <div style={{
                    color: '#e0e0e0',
                    background: '#0a0a0a',
                    padding: '0.75rem',
                    'border-radius': '4px',
                    'white-space': 'pre-wrap',
                    'word-break': 'break-word',
                    'max-height': '300px',
                    'overflow-y': 'auto'
                  }}>
                    {formatPayload(message.payload)}
                  </div>

                  <div style={{
                    'margin-top': '0.5rem',
                    color: '#666',
                    'font-size': '0.75rem',
                    display: 'flex',
                    gap: '1rem'
                  }}>
                    <span>ID: {message.id.substring(0, 8)}...</span>
                    <span>From: {message.from.id}</span>
                  </div>
                </div>
              )}
            </For>
          )}
        </div>
      </div>

      <div class="card">
        <h2>Message Statistics</h2>
        <div style={{ display: 'grid', 'grid-template-columns': 'repeat(auto-fit, minmax(200px, 1fr))', gap: '1rem' }}>
          <div style={{ background: '#1a1a1a', padding: '1rem', 'border-radius': '6px' }}>
            <div style={{ color: '#999', 'font-size': '0.85rem', 'margin-bottom': '0.25rem' }}>
              Total Messages
            </div>
            <div style={{ color: '#4a9eff', 'font-size': '1.5rem', 'font-weight': 'bold' }}>
              {messages().length}
            </div>
          </div>

          <div style={{ background: '#1a1a1a', padding: '1rem', 'border-radius': '6px' }}>
            <div style={{ color: '#999', 'font-size': '0.85rem', 'margin-bottom': '0.25rem' }}>
              Broadcasts
            </div>
            <div style={{ color: '#ff9800', 'font-size': '1.5rem', 'font-weight': 'bold' }}>
              {messages().filter(m => m.to === '*').length}
            </div>
          </div>

          <div style={{ background: '#1a1a1a', padding: '1rem', 'border-radius': '6px' }}>
            <div style={{ color: '#999', 'font-size': '0.85rem', 'margin-bottom': '0.25rem' }}>
              Direct Messages
            </div>
            <div style={{ color: '#66bb6a', 'font-size': '1.5rem', 'font-weight': 'bold' }}>
              {messages().filter(m => m.to !== '*').length}
            </div>
          </div>

          <div style={{ background: '#1a1a1a', padding: '1rem', 'border-radius': '6px' }}>
            <div style={{ color: '#999', 'font-size': '0.85rem', 'margin-bottom': '0.25rem' }}>
              Stream Status
            </div>
            <div style={{
              color: paused() ? '#ef5350' : '#66bb6a',
              'font-size': '1.5rem',
              'font-weight': 'bold'
            }}>
              {paused() ? 'Paused' : 'Live'}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
};

export default MessageStream;

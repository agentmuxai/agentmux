import { Component, createSignal, onMount, onCleanup, For, Show } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

interface AgentMessage {
  id: string;
  from: {
    id: string;
    name: string;
  };
  to: string;
  payload: {
    text: string;
  };
  timestamp: string;
  priority: string;
}

const MessageStream: Component = () => {
  const [messages, setMessages] = createSignal<AgentMessage[]>([]);
  const [paused, setPaused] = createSignal(false);
  const [filter, setFilter] = createSignal('');
  const [maxMessages, setMaxMessages] = createSignal(100);
  const [watcherStatus, setWatcherStatus] = createSignal<'stopped' | 'running' | 'error'>('stopped');
  const [replyTo, setReplyTo] = createSignal<AgentMessage | null>(null);
  const [replyText, setReplyText] = createSignal('');

  const formatTimestamp = (timestamp: string): string => {
    try {
      // Try parsing as ISO string first
      const date = new Date(timestamp);
      if (!isNaN(date.getTime())) {
        return date.toLocaleTimeString('en-US', {
          hour12: false,
          hour: '2-digit',
          minute: '2-digit',
          second: '2-digit'
        });
      }
      // Fallback: treat as Unix timestamp
      const unixDate = new Date(parseInt(timestamp) * 1000);
      return unixDate.toLocaleTimeString('en-US', {
        hour12: false,
        hour: '2-digit',
        minute: '2-digit',
        second: '2-digit'
      });
    } catch {
      return timestamp;
    }
  };

  const addMessage = (message: AgentMessage) => {
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
      msg.priority.toLowerCase().includes(filterText) ||
      msg.payload.text.toLowerCase().includes(filterText)
    );
  };

  const handleReply = (message: AgentMessage) => {
    setReplyTo(message);
    setReplyText('');
  };

  const sendReply = async () => {
    const msg = replyTo();
    if (!msg || !replyText().trim()) return;

    try {
      await invoke('send_message', {
        to: msg.from.id,
        message: replyText(),
        priority: 'normal',
      });

      setReplyTo(null);
      setReplyText('');
    } catch (err) {
      console.error('Failed to send reply:', err);
      alert(`Failed to send reply: ${err}`);
    }
  };

  const cancelReply = () => {
    setReplyTo(null);
    setReplyText('');
  };

  // Event listeners setup
  onMount(async () => {
    const unlisteners: UnlistenFn[] = [];

    try {
      // Start file watcher
      const result = await invoke<string>('start_file_watcher', {
        messagesDir: null,  // Use default ~/.agentmux/shared/messages
        agentId: 'AgentX',  // TODO: Make configurable
      });
      console.log('[MessageStream] File watcher started:', result);
      setWatcherStatus('running');

      // Listen for received messages (from file watcher)
      unlisteners.push(await listen<AgentMessage>('message_received', (event) => {
        console.log('[MessageStream] Message received:', event.payload);
        addMessage(event.payload);
      }));

      // Listen for sent messages (from send_message command)
      unlisteners.push(await listen('message_sent', (event) => {
        console.log('[MessageStream] Message sent event:', event.payload);
        const payload = event.payload as {
          from_agent: string;
          to_agent: string;
          message_text: string;
          timestamp: string;
        };

        // Create a message object to display in stream
        const sentMessage: AgentMessage = {
          id: `sent-${Date.now()}`,
          from: {
            id: payload.from_agent,
            name: payload.from_agent
          },
          to: payload.to_agent,
          payload: {
            text: payload.message_text
          },
          timestamp: payload.timestamp,
          priority: 'normal'
        };

        addMessage(sentMessage);
      }));

    } catch (err) {
      console.error('[MessageStream] Failed to start file watcher:', err);
      setWatcherStatus('error');
    }

    // Cleanup all event listeners
    onCleanup(async () => {
      try {
        unlisteners.forEach(fn => fn());
        await invoke('stop_file_watcher');
        setWatcherStatus('stopped');
      } catch (err) {
        console.error('[MessageStream] Failed to stop file watcher:', err);
      }
    });
  });

  return (
    <div>
      <div class="card">
        <div style={{ display: 'flex', 'justify-content': 'space-between', 'align-items': 'center', 'margin-bottom': '1rem' }}>
          <div>
            <h2>Message Stream</h2>
            <div style={{
              color: watcherStatus() === 'running' ? '#66bb6a' : watcherStatus() === 'error' ? '#ef5350' : '#999',
              'font-size': '0.85rem',
              'margin-top': '0.25rem'
            }}>
              Watcher: {watcherStatus()}
            </div>
          </div>
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
                          background: message.priority === 'urgent' ? '#4a2a2a' : '#2a2a4a',
                          color: message.priority === 'urgent' ? '#ff9c9c' : '#9c9cff',
                          padding: '0.25rem 0.5rem',
                          'border-radius': '4px',
                          'font-size': '0.75rem',
                          'font-weight': 'bold'
                        }}
                      >
                        {message.priority}
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
                    'overflow-y': 'auto',
                    'margin-bottom': '0.5rem'
                  }}>
                    {message.payload.text}
                  </div>

                  <div style={{
                    display: 'flex',
                    'justify-content': 'space-between',
                    'align-items': 'center'
                  }}>
                    <div style={{
                      color: '#666',
                      'font-size': '0.75rem',
                      display: 'flex',
                      gap: '1rem'
                    }}>
                      <span>ID: {message.id.substring(0, 8)}...</span>
                      <span>From: {message.from.id}</span>
                    </div>
                    <button
                      onClick={() => handleReply(message)}
                      style={{
                        background: '#2a4a2a',
                        color: '#9cff9c',
                        border: 'none',
                        padding: '0.35rem 0.75rem',
                        'border-radius': '4px',
                        cursor: 'pointer',
                        'font-size': '0.75rem',
                        'font-weight': 'bold'
                      }}
                    >
                      💬 Reply
                    </button>
                  </div>
                </div>
              )}
            </For>
          )}
        </div>
      </div>

      {/* Reply Modal */}
      <Show when={replyTo()}>
        <div style={{
          position: 'fixed',
          top: 0,
          left: 0,
          right: 0,
          bottom: 0,
          background: 'rgba(0, 0, 0, 0.8)',
          display: 'flex',
          'align-items': 'center',
          'justify-content': 'center',
          'z-index': 1000
        }}>
          <div class="card" style={{
            'max-width': '600px',
            width: '90%',
            'max-height': '80vh',
            'overflow-y': 'auto'
          }}>
            <h2>Reply to {replyTo()?.from.name}</h2>

            <div style={{
              background: '#1a1a1a',
              padding: '1rem',
              'border-radius': '6px',
              'margin-bottom': '1rem',
              border: '1px solid #2a2a2a'
            }}>
              <div style={{
                color: '#999',
                'font-size': '0.85rem',
                'margin-bottom': '0.5rem'
              }}>
                Original message:
              </div>
              <div style={{ color: '#e0e0e0' }}>
                {replyTo()?.payload.text}
              </div>
            </div>

            <textarea
              value={replyText()}
              onInput={(e) => setReplyText(e.currentTarget.value)}
              placeholder="Type your reply..."
              style={{
                width: '100%',
                'min-height': '150px',
                background: '#1a1a1a',
                border: '1px solid #3a3a3a',
                color: '#e0e0e0',
                padding: '1rem',
                'border-radius': '6px',
                'font-family': 'inherit',
                'font-size': '0.95rem',
                'margin-bottom': '1rem',
                resize: 'vertical'
              }}
            />

            <div style={{
              display: 'flex',
              gap: '0.5rem',
              'justify-content': 'flex-end'
            }}>
              <button
                class="secondary"
                onClick={cancelReply}
                style={{ padding: '0.75rem 1.5rem' }}
              >
                Cancel
              </button>
              <button
                onClick={sendReply}
                disabled={!replyText().trim()}
                style={{
                  padding: '0.75rem 1.5rem',
                  background: replyText().trim() ? '#2a4a2a' : '#1a1a1a',
                  color: replyText().trim() ? '#9cff9c' : '#666',
                  border: 'none',
                  'border-radius': '6px',
                  cursor: replyText().trim() ? 'pointer' : 'not-allowed',
                  'font-weight': 'bold'
                }}
              >
                Send Reply
              </button>
            </div>
          </div>
        </div>
      </Show>

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

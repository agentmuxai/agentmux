import WebSocket from 'ws';

const AGENTMUX_URL = 'wss://34.192.240.117:8443';
const TOKEN = 'eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJhZ2VudElkIjoiYWdlbnR4IiwiaWF0IjoxNzY3NTc3MjY2LCJleHAiOjE3OTkxMTMyNjZ9.AosjMmiPZ2YMzah3JgIk-ZHHhdzApTVms37KZM81adA';

console.log('Connecting to AgentMux server...');
const ws = new WebSocket(`${AGENTMUX_URL}?token=${TOKEN}`, {
  rejectUnauthorized: false // Accept self-signed certificates
});

ws.on('open', () => {
  console.log('✓ Connected to AgentMux server as agentx');

  // Send a message to agent5
  const message = {
    type: 'send_message',
    to: 'agent5',
    message: 'Hello from AgentX! Testing AgentMux cloud infrastructure.',
    priority: 'normal',
    requestId: Date.now().toString()
  };

  console.log('Sending message to agent5:', message.message);
  ws.send(JSON.stringify(message));
});

ws.on('message', (data) => {
  try {
    const response = JSON.parse(data.toString());
    console.log('Received response:', JSON.stringify(response, null, 2));

    // Close after receiving response
    setTimeout(() => {
      ws.close();
      process.exit(0);
    }, 1000);
  } catch (error) {
    console.error('Failed to parse response:', error);
  }
});

ws.on('error', (error) => {
  console.error('WebSocket error:', error.message);
  process.exit(1);
});

ws.on('close', () => {
  console.log('Disconnected from AgentMux server');
});

// Timeout after 10 seconds
setTimeout(() => {
  console.log('Timeout - closing connection');
  ws.close();
  process.exit(1);
}, 10000);

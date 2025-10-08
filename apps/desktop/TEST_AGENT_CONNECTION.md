# Testing Agent Connections

## ✅ Desktop App is Now Functional!

The WebSocket server is now fully implemented. Here's how to test it:

---

## 🚀 Step 1: Start the Desktop App

If not already running:

```bash
cd D:/Code/WebProjects/agentmux/apps/desktop
npm run tauri:dev
```

---

## 🎮 Step 2: Start the Bus

In the desktop app:
1. Click the **"▶️ Start Bus"** button on the Dashboard tab
2. You should see:
   - Status changes to "Running" (green dot)
   - "Bus Status" card shows ✓
   - Message: "Bus is running. Agents can connect at ws://localhost:8765/ws"

Console output (in terminal):
```
🚀 AgentMux Bus starting on localhost:8765
```

---

## 🤖 Step 3: Connect an Agent (Using CLI)

Open a **new terminal** and connect using the existing AgentMux CLI:

```bash
# Navigate to any workspace (e.g., WebProjects)
cd D:/Code/WebProjects

# Create a simple test script to connect
cat > test-agent.js << 'EOF'
const WebSocket = require('ws');

// Create agent identity
const identity = {
  id: `TestAgent-${process.pid}-${Date.now()}`,
  name: 'TestAgent',
  workspace: process.cwd(),
  pid: process.pid,
  started_at: Date.now()
};

console.log('Connecting to AgentMux Desktop...');
console.log('Identity:', identity);

const ws = new WebSocket('ws://localhost:8765/ws');

ws.on('open', () => {
  console.log('✅ Connected to bus!');

  // Register with identity
  ws.send(JSON.stringify(identity));

  // Send a test message
  setTimeout(() => {
    const msg = {
      id: `msg-${Date.now()}`,
      from: identity,
      to: '*',
      msg_type: 'message',
      payload: { text: 'Hello from test agent!' },
      timestamp: Date.now()
    };
    ws.send(JSON.stringify(msg));
    console.log('📤 Sent message');
  }, 1000);
});

ws.on('message', (data) => {
  const msg = JSON.parse(data.toString());
  console.log('📨 Received:', msg);
});

ws.on('close', () => {
  console.log('❌ Disconnected');
});

ws.on('error', (err) => {
  console.error('Error:', err);
});

// Keep alive
setInterval(() => {
  console.log('Heartbeat...');
}, 5000);
EOF

# Run the test agent (needs Node.js with ws package)
npm install ws
node test-agent.js
```

---

## 🔍 Step 4: Verify in Desktop App

Switch back to the AgentMux Desktop window:

### Dashboard Tab
- **Connected Agents** should show: `1`
- **Messages/sec** will update when messages flow
- **Bus Status** shows ✓

### Agents Tab
- You should see your `TestAgent-<pid>-<timestamp>` listed
- Status: `online` (green dot)
- Workspace path shown
- Messages sent/received counters
- Uptime counting up

---

## 🧪 Alternative: Use AgentMux CLI (Needs Update)

The existing CLI needs a small update to connect to the desktop WebSocket server instead of file-based transport.

**Quick test without CLI:**

```bash
# Using websocat (WebSocket CLI tool)
# Install: cargo install websocat

# Connect and register
echo '{"id":"CLI-Agent-123","name":"CLIAgent","workspace":"/test","pid":123,"started_at":1234567890}' | websocat ws://localhost:8765/ws
```

---

## 📊 What You Should See

### Desktop App Output (Terminal)
```
🚀 AgentMux Bus starting on localhost:8765
✅ Agent connected: TestAgent-12345-1759900000
📨 Message from TestAgent-12345-1759900000
```

### Test Agent Output
```
Connecting to AgentMux Desktop...
Identity: { id: 'TestAgent-12345-1759900000', name: 'TestAgent', ... }
✅ Connected to bus!
📤 Sent message
Heartbeat...
```

### Desktop UI
- Dashboard shows 1 connected agent
- Agents tab lists your test agent
- Real-time updates every 2 seconds

---

## 🎉 Success Criteria

✅ You can click "Start Bus" and it actually starts (check terminal for "🚀 AgentMux Bus starting")
✅ A WebSocket client can connect to `ws://localhost:8765/ws`
✅ Agent appears in the Agents tab
✅ Agent count updates in Dashboard
✅ Messages sent/received counters work
✅ Clicking "Stop Bus" shuts down the server

---

## 🐛 Troubleshooting

**"Bus is already running" error:**
- Click "Stop Bus" first, then "Start Bus"

**Agent doesn't appear:**
- Make sure you sent the identity JSON as the first message
- Check terminal for connection logs

**"Connection refused":**
- Verify bus is actually started (green dot on Dashboard)
- Check if port 8765 is already in use: `netstat -an | findstr :8765`

---

## 🚀 Next Steps

Now that the desktop app is working, you can:

1. **Update the AgentMux CLI** to connect to WebSocket instead of file-based transport
2. **Add message stream visualization** in the desktop app
3. **Implement topology graph** using D3.js
4. **Add performance charts** for metrics over time

---

**The bus is live! Try it now!** 🎊

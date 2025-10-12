#!/bin/bash
# Manual test script for AgentMux wrapper

echo "=== AgentMux Wrapper Manual Test ==="
echo ""

# Check if built
if [ ! -f "dist/index.js" ]; then
  echo "❌ Wrapper not built. Run: npm run build"
  exit 1
fi

echo "✅ Wrapper is built"
echo ""

# Check messages directory
MESSAGES_DIR="$HOME/.agentmux/shared/messages"

if [ ! -d "$MESSAGES_DIR" ]; then
  echo "📁 Creating messages directory: $MESSAGES_DIR"
  mkdir -p "$MESSAGES_DIR"
fi

echo "✅ Messages directory exists: $MESSAGES_DIR"
echo ""

# Test 1: Check if claude CLI is available
echo "=== Test 1: Check Claude CLI ==="
if command -v claude &> /dev/null; then
  echo "✅ Claude CLI found"
  echo "   Version: $(claude --version 2>&1 | head -1)"
else
  echo "⚠️  Claude CLI not found"
  echo "   The wrapper will fail to start without it"
fi
echo ""

# Test 2: Check dependencies
echo "=== Test 2: Check Dependencies ==="
cd "$(dirname "$0")"

if [ -d "node_modules/chokidar" ]; then
  echo "✅ chokidar installed"
else
  echo "❌ chokidar not installed. Run: npm install"
  exit 1
fi

if [ -d "node_modules/commander" ]; then
  echo "✅ commander installed"
else
  echo "❌ commander not installed. Run: npm install"
  exit 1
fi

# Check for node-pty (optional)
if [ -d "../../node_modules/node-pty" ] || [ -d "node_modules/node-pty" ]; then
  echo "✅ node-pty installed (optional)"
else
  echo "⚠️  node-pty not installed (optional, required for full functionality)"
  echo "   On Windows: npm install -g windows-build-tools && npm install node-pty"
  echo "   On Linux: sudo apt-get install build-essential python3 && npm install node-pty"
fi
echo ""

# Test 3: Create test message
echo "=== Test 3: Create Test Message ==="

TEST_MESSAGE=$(cat <<EOF
{
  "id": "test-$(date +%s)",
  "from": {
    "id": "TestSender",
    "name": "TestSender"
  },
  "to": "AgentX-*",
  "payload": {
    "text": "This is a test message"
  },
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "priority": "normal"
}
EOF
)

TEST_FILE="$MESSAGES_DIR/test-$(date +%s).json"
echo "$TEST_MESSAGE" > "$TEST_FILE"

echo "✅ Created test message: $TEST_FILE"
echo "   Content:"
cat "$TEST_FILE" | sed 's/^/   /'
echo ""

# Test 4: Usage instructions
echo "=== Test 4: Manual Testing Instructions ==="
echo ""
echo "To test the wrapper manually:"
echo ""
echo "1. Start wrapper in Terminal 1:"
echo "   $ AGENT_ID=AgentX node dist/index.js claude"
echo ""
echo "2. Send test message in Terminal 2:"
echo "   $ cat <<'EOF' > $HOME/.agentmux/shared/messages/msg-test.json"
echo "   {"
echo "     \"id\": \"msg-test\","
echo "     \"from\": {\"id\": \"Agent1\", \"name\": \"Agent1\"},"
echo "     \"to\": \"AgentX-*\","
echo "     \"payload\": {\"text\": \"Test notification\"},"
echo "     \"timestamp\": \"$(date -u +%Y-%m-%dT%H:%M:%SZ)\""
echo "   }"
echo "   EOF"
echo ""
echo "3. Expected result in Terminal 1:"
echo "   You should see a blue highlighted bar:"
echo "   ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "    📨 Remote message from Agent1"
echo "   ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

# Test 5: Via AgentMux CLI
echo "=== Test 5: Via AgentMux CLI (Recommended) ==="
echo ""
echo "From agentmux root directory:"
echo ""
echo "1. Build CLI:"
echo "   $ cd apps/cli && npm run build"
echo ""
echo "2. Use agentmux wrap command:"
echo "   $ node apps/cli/dist/index.js wrap claude --agent-id AgentX --debug"
echo ""
echo "3. Send message:"
echo "   $ node apps/cli/dist/index.js send \"AgentX-*\" \"Test message\""
echo ""

echo "=== Summary ==="
echo ""
echo "✅ Wrapper is built and ready to test"
echo "✅ Messages directory exists"
echo "✅ Dependencies installed"
echo ""
echo "Next steps:"
echo "1. Install node-pty (optional, for full functionality)"
echo "2. Start wrapper: AGENT_ID=AgentX node dist/index.js claude"
echo "3. Send test message from another terminal"
echo ""

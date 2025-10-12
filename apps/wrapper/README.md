# @agentmux/wrapper

Reactive wrapper for AI CLIs enabling supervised agent-to-agent communication with human oversight.

## Features

- ✅ **Reactive Notifications**: < 100ms latency for message detection
- ✅ **Human Supervision**: All interactions visible on terminal
- ✅ **Visual Highlighting**: Blue/red ANSI colors for message notifications
- ✅ **PTY Integration**: Full terminal emulation with stdin injection
- ✅ **Cost-Free**: No API calls, works offline
- ✅ **Extensible**: Support for multiple AI CLIs (Claude, Gemini, GPT, etc.)

## Installation

```bash
# From agentmux monorepo root
npm install
npm run build
```

## Usage

### Via AgentMux CLI (Recommended)

```bash
# Wrap Claude CLI
agentmux wrap claude

# With custom agent ID
agentmux wrap claude --agent-id Agent3

# With debug logging
agentmux wrap claude --debug

# With custom messages directory
agentmux wrap claude --messages-dir ~/.custom/messages
```

### Standalone

```bash
# From wrapper package
cd apps/wrapper
npm run build

# Run directly
AGENT_ID=AgentX node dist/index.js claude
```

## How It Works

### 1. PTY Wrapper

The wrapper spawns the AI CLI in a pseudo-terminal (PTY):

```typescript
ptyProcess = pty.spawn('claude', [], {
  name: 'xterm-color',
  cols: 80,
  rows: 30
});
```

### 2. Message Watcher

Monitors `~/.agentmux/shared/messages/` for new JSON files:

```typescript
chokidar.watch('~/.agentmux/shared/messages/*.json')
  .on('add', handleNewMessage);
```

### 3. Notification Injection

When a message arrives addressed to this agent:

```typescript
// Show blue highlighted notification
ptyProcess.write('\x1b[44m\x1b[1m 📨 Remote message from Agent1 \x1b[0m\n');

// Inject check command
ptyProcess.write('check messages\n');
```

### 4. Human Oversight

- Human sees the blue notification appear on terminal
- Human sees agent read the message
- Human can intervene at any point
- Human approves or redirects agent's response

## Architecture

```
Message Bus (file)
    ↓
MessageWatcher detects new file
    ↓
BaseWrapper.handleMessage()
    ↓
Show blue notification (human sees)
    ↓
Inject "check messages" command
    ↓
Agent reads message via MCP tool
    ↓
Human monitors and approves
```

## Message Format

Messages are JSON files in `~/.agentmux/shared/messages/`:

```json
{
  "id": "msg-1234567890",
  "from": {
    "id": "Agent1",
    "name": "Agent1"
  },
  "to": "AgentX-*",
  "payload": {
    "text": "Review PR #156"
  },
  "timestamp": "2025-10-11T21:00:00Z",
  "priority": "normal"
}
```

### Message Routing

- **Direct**: `"to": "AgentX"` - Exact match
- **Pattern**: `"to": "AgentX-*"` - Prefix match
- **Broadcast**: `"to": "*"` - All agents

### Priority Levels

- **normal**: Blue background `\x1b[44m` + 📨 icon
- **urgent**: Red background `\x1b[41m` + ⚠️ icon

## Testing

```bash
# Run all tests
npm test

# Watch mode
npm run test:watch

# Coverage report
npm run test:coverage
```

### Unit Tests

- `watcher.test.ts` - Message routing, pattern matching, duplicate prevention
- `base-wrapper.test.ts` - Notification display, command injection, lifecycle

### Integration Tests

- `integration.test.ts` - End-to-end message flow with PTY wrapper

## Configuration

### Environment Variables

- `AGENT_ID` - Agent identifier (default: `AgentX`)
- `DEBUG` - Enable debug logging (default: `false`)

### Options

```typescript
interface WrapperOptions {
  agentId?: string;          // Agent ID
  messagesDir?: string;      // Custom messages directory
  debug?: boolean;           // Debug logging
}
```

## Example Workflow

### Terminal 1 (AgentX)

```bash
$ agentmux wrap claude
🔄 Starting wrapper for claude...
  Agent ID: AgentX

User: "Request code review from Agent3"
AgentX: "Sending review request..."
```

### Terminal 4 (Agent3)

```bash
$ agentmux wrap claude
🔄 Starting wrapper for claude...
  Agent ID: Agent3

[Working on something else...]

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
 📨 Remote message from AgentX
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Agent3: "I received a message from AgentX: 'Please review PR #156'"
Agent3: "Should I review now or continue current task?"

User: "Review now"

Agent3: "Fetching PR... reviewing... done. Posted 3 comments."
```

## Extending to Other CLIs

Create a new wrapper class:

```typescript
// apps/wrapper/src/wrappers/gemini.ts
import { BaseWrapper } from './base';

export class GeminiWrapper extends BaseWrapper {
  get command(): string {
    return 'gemini';  // or 'gcloud ai'
  }

  // Gemini-specific customizations
}
```

Update CLI integration:

```typescript
// apps/cli/src/index.ts
case 'gemini':
  wrapper = new GeminiWrapper(options);
  break;
```

## Troubleshooting

### "PTY process not initialized"

Ensure `start()` is called before `inject()`:

```typescript
await wrapper.start();
wrapper.inject('command');  // OK
```

### Messages not detected

Check messages directory exists:

```bash
ls ~/.agentmux/shared/messages/
```

Verify file watcher is running:

```bash
agentmux wrap claude --debug
```

### Terminal not responding

Ensure stdin is in raw mode. Wrapper handles this automatically.

## Development

```bash
# Build
npm run build

# Watch mode
npm run dev

# Run tests
npm test
```

## License

MIT

## Contributing

See main agentmux [CONTRIBUTING.md](../../CONTRIBUTING.md)

# AgentMux Integration Guide

How to integrate AgentMux with Claude Code, Gemini CLI, and other AI assistants.

---

## Automatic Agent Registration

When you start Claude, Gemini, or any AI assistant, you want them to automatically:
1. Register with AgentMux
2. Start listening for messages
3. Auto-respond to commands

---

## Option 1: Shell Hook (Recommended)

### For Claude Code (Windows)

Add to your PowerShell profile or create a startup script:

**`_scripts/agent-startup.ps1`**

```powershell
# AgentMux Auto-Registration
# Run this when starting a new Claude Code session

$AGENTMUX_CLI = "D:\Code\WebProjects\agentmux\apps\cli\dist\index.js"
$WORKSPACE = Get-Location

Write-Host "🤖 Starting AgentMux listener..." -ForegroundColor Cyan

# Start listener in background
$job = Start-Job -ScriptBlock {
    param($cli, $workspace)
    Set-Location $workspace
    node $cli listen
} -ArgumentList $AGENTMUX_CLI, $WORKSPACE

# Store job ID
$env:AGENTMUX_JOB = $job.Id

Write-Host "✓ AgentMux listener started (Job ID: $($job.Id))" -ForegroundColor Green
Write-Host "  Stop with: Stop-Job $($job.Id)" -ForegroundColor Gray

# Broadcast registration
Start-Sleep -Seconds 2
node $AGENTMUX_CLI send "*" "Agent registered and listening" --type status
```

**Usage:**
```powershell
# Add to your session startup
. D:\Code\WebProjects\_scripts\agent-startup.ps1
```

### For Claude Code (Bash/Linux/Mac)

Add to `~/.bashrc` or `~/.zshrc`:

```bash
# AgentMux Auto-Registration
AGENTMUX_CLI="$HOME/Code/WebProjects/agentmux/apps/cli/dist/index.js"

# Start listener in background
if [ ! -f /tmp/agentmux-listener-$$.pid ]; then
    nohup node "$AGENTMUX_CLI" listen > /tmp/agentmux-$$.log 2>&1 &
    echo $! > /tmp/agentmux-listener-$$.pid
    echo "🤖 AgentMux listener started (PID: $!)"

    # Give it time to start
    sleep 2

    # Broadcast registration
    node "$AGENTMUX_CLI" send "*" "Agent registered and listening" --type status
fi

# Cleanup on exit
trap 'kill $(cat /tmp/agentmux-listener-$$.pid 2>/dev/null) 2>/dev/null; rm /tmp/agentmux-listener-$$.pid 2>/dev/null' EXIT
```

---

## Option 2: VS Code Task (For Claude Code Extension)

**`.vscode/tasks.json`**

```json
{
  "version": "2.0.0",
  "tasks": [
    {
      "label": "Start AgentMux Listener",
      "type": "shell",
      "command": "node",
      "args": [
        "D:/Code/WebProjects/agentmux/apps/cli/dist/index.js",
        "listen"
      ],
      "isBackground": true,
      "problemMatcher": [],
      "presentation": {
        "reveal": "silent",
        "panel": "dedicated"
      },
      "runOptions": {
        "runOn": "folderOpen"
      }
    }
  ]
}
```

This will automatically start the listener when you open the workspace.

---

## Option 3: Manual Registration

### When Starting Your Session

```bash
# Start listener (keeps running)
node D:/Code/WebProjects/agentmux/apps/cli/dist/index.js listen &

# Or in a separate terminal
node D:/Code/WebProjects/agentmux/apps/cli/dist/index.js listen
```

### When Ending Your Session

The listener will automatically send a shutdown notification when stopped (Ctrl+C).

---

## Option 4: Systemd Service (Linux)

**`~/.config/systemd/user/agentmux.service`**

```ini
[Unit]
Description=AgentMux Listener
After=network.target

[Service]
Type=simple
WorkingDirectory=%h/Code/WebProjects
ExecStart=/usr/bin/node %h/Code/WebProjects/agentmux/apps/cli/dist/index.js listen
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
```

**Enable:**
```bash
systemctl --user enable agentmux
systemctl --user start agentmux
systemctl --user status agentmux
```

---

## Integration with AI Assistants

### Claude Code

**Add to `.claude/CLAUDE.md`:**

```markdown
## AgentMux Integration

When starting a new session:

1. Start AgentMux listener:
   ```bash
   node D:/Code/WebProjects/agentmux/apps/cli/dist/index.js listen &
   ```

2. Listener runs in background and auto-responds to:
   - `whoami` - Identify yourself
   - `ping` - Connection test
   - `status` - Get agent status

3. To communicate with other agents:
   ```bash
   node D:/Code/WebProjects/agentmux/apps/cli/dist/index.js send "*" "Your message"
   ```
```

### Gemini CLI

**Wrapper script: `gemini-with-agentmux.sh`**

```bash
#!/bin/bash

# Start AgentMux listener
AGENTMUX_CLI="$HOME/Code/WebProjects/agentmux/apps/cli/dist/index.js"
node "$AGENTMUX_CLI" listen > /tmp/agentmux-gemini-$$.log 2>&1 &
LISTENER_PID=$!

echo "🤖 AgentMux listener started (PID: $LISTENER_PID)"

# Cleanup on exit
trap "kill $LISTENER_PID 2>/dev/null" EXIT

# Run Gemini CLI
gemini "$@"
```

**Usage:**
```bash
chmod +x gemini-with-agentmux.sh
./gemini-with-agentmux.sh
```

### Cursor / Windsurf / Other IDEs

Same approach - start the listener in a terminal or as a background task.

---

## Checking If Agent Is Registered

### From Any Terminal

```bash
# Send whoami command
node D:/Code/WebProjects/agentmux/apps/cli/dist/index.js send "*" "whoami" --type command

# If agents are listening, they'll respond
```

### Check Running Listeners

**Windows (PowerShell):**
```powershell
Get-Process node | Where-Object { $_.CommandLine -like "*agentmux*listen*" }
```

**Linux/Mac:**
```bash
ps aux | grep "agentmux.*listen"
```

---

## Message Bus Location

All agents share: `D:\Code\WebProjects\_temp\agentmux-bus\`

- `inbox/` - Received messages
- `outbox/` - Sent messages

**Ensure this directory exists:**
```bash
mkdir -p D:/Code/WebProjects/_temp/agentmux-bus/{inbox,outbox}
```

---

## Best Practices

### 1. Start Listener First
Always start the listener before doing any work:
```bash
node D:/Code/WebProjects/agentmux/apps/cli/dist/index.js listen &
```

### 2. Use Broadcast for Discovery
```bash
node D:/Code/WebProjects/agentmux/apps/cli/dist/index.js send "*" "whoami" --type command
```

### 3. Keep Listener Running
Use background process or separate terminal.

### 4. Clean Shutdown
Use Ctrl+C to stop listener - it sends a shutdown notification.

---

## Environment Variables

```bash
# Override agent name
export AGENT_NAME="CustomAgent1"

# Message bus path (default: _temp/agentmux-bus)
export AGENTMUX_BUS_PATH="/custom/path/to/bus"
```

---

## Quick Setup Commands

### Windows (PowerShell)

```powershell
# Create startup script
@"
`$AGENTMUX_CLI = "D:\Code\WebProjects\agentmux\apps\cli\dist\index.js"
Write-Host "🤖 Starting AgentMux..." -ForegroundColor Cyan
Start-Job -ScriptBlock { node `$using:AGENTMUX_CLI listen }
Start-Sleep 2
node `$AGENTMUX_CLI send "*" "Agent online" --type status
"@ | Out-File -FilePath ~\agentmux-start.ps1

# Run on session start
. ~\agentmux-start.ps1
```

### Linux/Mac (Bash)

```bash
# Create startup script
cat > ~/agentmux-start.sh <<'EOF'
#!/bin/bash
AGENTMUX_CLI="$HOME/Code/WebProjects/agentmux/apps/cli/dist/index.js"
echo "🤖 Starting AgentMux..."
nohup node "$AGENTMUX_CLI" listen > /tmp/agentmux-$$.log 2>&1 &
sleep 2
node "$AGENTMUX_CLI" send "*" "Agent online" --type status
EOF

chmod +x ~/agentmux-start.sh

# Add to ~/.bashrc
echo ". ~/agentmux-start.sh" >> ~/.bashrc
```

---

## Troubleshooting

**Q: Listener not starting?**

A: Check if port/directory is accessible:
```bash
ls -la D:/Code/WebProjects/_temp/agentmux-bus/
```

**Q: Multiple listeners for same agent?**

A: Only one listener per agent. Kill existing:
```bash
# Windows
Stop-Job <JobID>

# Linux
kill $(ps aux | grep "agentmux.*listen" | awk '{print $2}')
```

**Q: Messages not received?**

A: Ensure all agents use the same bus directory:
```bash
D:\Code\WebProjects\_temp\agentmux-bus\
```

---

## Next Steps

1. Choose integration method (Shell hook recommended)
2. Test with `whoami` command
3. Configure auto-start for your IDE/terminal
4. Start communicating between agents!

🚀 **Your AI assistants can now talk to each other!**

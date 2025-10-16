# Agent Spawning Fix Specification

**Date:** 2025-10-15
**Issue:** E2E tests fail because spawned Claude agents terminate immediately
**Status:** Investigation & Fix Required

---

## Problem Statement

E2E tests TC6-TC9 fail because the embedded Claude agent process terminates immediately after spawning. The terminal shows:
```
[ERROR] [PROCESS] [TestAgent] Process terminated by signal
[INFO] [STDIN] [TestAgent] PTY stdin handler ended (total inputs: 2)
[INFO] [STDOUT] [TestAgent] → D:\2004h
```

**Evidence:**
- Screenshot: `tc6-01-before-message-1760559169566.png` shows terminated process
- Debug console shows PTY process monitor ended
- Terminal displays "Enter to confirm, ↑↓ to navigate, Esc to clear" (Claude CLI prompt)
- No active shell to receive commands

---

## Root Cause Analysis

### Current Test Configuration
```javascript
// tests/e2e/claude-terminal-interaction.spec.js:34
workspacePath: 'D:\\Code\\PythonProjects',
label: 'TestAgent',
```

### Potential Issues

1. **Invalid Workspace Path**
   - Path `D:\Code\PythonProjects` may not exist or may be empty
   - Claude CLI may fail initialization if workspace has no `.git` or no valid project
   - No `.claude` configuration in that directory

2. **Claude CLI Not Found**
   - `claude` command may not be in PATH during E2E test execution
   - Tauri app running in isolated environment may not inherit shell PATH
   - Release build may have different environment than dev mode

3. **Missing Environment Variables**
   - `ANTHROPIC_API_KEY` may not be set during test execution
   - `AGENTMUX_DISABLE_SINGLE_INSTANCE=1` is set, but other env vars may be missing

4. **Permission Issues**
   - PTY may not have permission to spawn processes in release build
   - Windows security settings may block subprocess creation

5. **Working Directory Issue**
   - Agent spawns but can't `cd` to specified workspace
   - Invalid path format (forward vs backward slashes)

---

## Recommended Fixes

### Fix 1: Use Known Valid Workspace (IMMEDIATE)

**Change test to use agentmux repo itself:**

```javascript
// tests/e2e/claude-terminal-interaction.spec.js
before(async () => {
  await waitForAppReady();

  // Use the agentmux desktop directory itself (we know this exists!)
  const testWorkspace = 'D:\\Code\\WebProjects\\agentmux\\apps\\desktop';

  agentLabel = await spawnClaudeAgent({
    workspacePath: testWorkspace,
    label: 'E2ETestAgent',
  });

  await selectAgent(agentLabel);
  await waitForTerminalConnected();
});
```

**Why this works:**
- Path definitely exists (we're running from it!)
- Has `.git` directory (valid git repo)
- Has project files (not empty)
- Already configured for Claude

### Fix 2: Create Temporary Test Workspace

**Create minimal test directory during E2E setup:**

```javascript
// wdio.conf.js - onPrepare hook
onPrepare: async function () {
  const fs = require('fs');
  const path = require('path');

  // Create temporary test workspace
  const testWorkspace = path.join(__dirname, 'test-workspace');
  if (!fs.existsSync(testWorkspace)) {
    fs.mkdirSync(testWorkspace, { recursive: true });

    // Initialize as git repo (Claude likes this)
    spawnSync('git', ['init'], { cwd: testWorkspace });

    // Create a simple file
    fs.writeFileSync(
      path.join(testWorkspace, 'README.md'),
      '# E2E Test Workspace\n'
    );
  }

  // Store path for tests to use
  global.testWorkspace = testWorkspace;

  // ... rest of build process
}
```

**Update test:**
```javascript
workspacePath: global.testWorkspace || 'D:\\Code\\WebProjects\\agentmux\\apps\\desktop',
```

### Fix 3: Verify Claude CLI Availability

**Add diagnostic check before spawning:**

```rust
// src-tauri/src/embedded_claude/process.rs
pub async fn spawn_embedded_claude(...) -> Result<ClaudeInstance, String> {
    // Check if claude command exists
    let claude_check = Command::new("claude")
        .arg("--version")
        .output()
        .await;

    if claude_check.is_err() {
        return Err("Claude CLI not found in PATH. Install with: npm install -g @anthropic-ai/claude-code".to_string());
    }

    // Check workspace exists
    if !Path::new(&workspace_path).exists() {
        return Err(format!("Workspace path does not exist: {}", workspace_path));
    }

    // Continue with spawn...
}
```

### Fix 4: Add Environment Variable Propagation

**Ensure test environment has necessary variables:**

```javascript
// wdio.conf.js - capabilities
capabilities: [{
  'tauri:options': {
    application: '...',
    // Explicitly pass environment variables
    environment: {
      ANTHROPIC_API_KEY: process.env.ANTHROPIC_API_KEY,
      PATH: process.env.PATH,
      HOME: process.env.HOME || process.env.USERPROFILE,
      AGENTMUX_DISABLE_SINGLE_INSTANCE: '1',
    },
  },
}],
```

### Fix 5: Add Error Logging to Agent Spawn

**Capture stderr from Claude process:**

```rust
// src-tauri/src/embedded_claude/process.rs
// In spawn_claude_pty function

// Create command with explicit error capture
let mut cmd = Command::new("claude");
cmd.current_dir(&workspace_path)
    .env("AGENTMUX_DISABLE_SINGLE_INSTANCE", "1")
    .stdout(Stdio::piped())
    .stderr(Stdio::piped()); // Capture stderr!

// After spawn, read stderr
let stderr_output = String::from_utf8_lossy(&child.stderr);
if !stderr_output.is_empty() {
    eprintln!("[CLAUDE_SPAWN] stderr: {}", stderr_output);
}
```

---

## Testing Strategy

### Step 1: Verify Workspace Exists
```bash
# Run this before E2E tests
ls -la "D:\Code\PythonProjects"

# If doesn't exist, create it
mkdir -p "D:\Code\PythonProjects\test-project"
cd "D:\Code\PythonProjects\test-project"
git init
echo "# Test" > README.md
```

### Step 2: Test Claude CLI Manually
```bash
# Check if claude is available
claude --version

# Try spawning in test workspace
cd "D:\Code\PythonProjects"
claude
# Should show Claude CLI prompt, not crash
```

### Step 3: Test with Dev Mode First
```bash
# Run app in dev mode (not release)
npm run tauri dev

# Manually spawn agent with test workspace
# See if it works in dev mode before trying E2E
```

### Step 4: Update Tests and Run
```bash
# After applying Fix 1 (use agentmux directory)
npm run test:e2e

# Should see agent spawn successfully
```

---

## Implementation Priority

### Immediate (Do First):
1. **Apply Fix 1** - Change test workspace to `agentmux/apps/desktop`
2. **Run E2E tests** - See if this alone fixes the issue

### If Still Failing:
3. **Apply Fix 3** - Add diagnostic checks to Rust spawn function
4. **Check logs** - See what specific error is happening

### If Still Failing:
5. **Apply Fix 4** - Ensure environment variables are passed
6. **Apply Fix 5** - Add stderr capture to see Claude's error messages

### Long-term:
7. **Apply Fix 2** - Create proper test workspace setup
8. **Add cleanup** - Remove test workspace after tests complete

---

## Expected Behavior After Fix

**Test output should show:**
```
[Test] ✓ Claude agent spawned: E2ETestAgent
[Test] ✓ Agent selected
[Test] ✓ Terminal connected

[Test] TC6: Testing message sending to agent
[E2E] Sending message to agent: "echo "Hello from E2E test""
[E2E] ✓ Message sent
[Test] ✅ TC6 PASSED: Message sent to agent

[Test] TC7: Testing agent response reception
[E2E] Sending message to agent: "pwd"
[E2E] Waiting for agent response containing: "Code"
[E2E] ✓ Agent response received
[Test] ✅ TC7 PASSED: Agent response received
```

**Screenshots should show:**
- Terminal with active shell prompt
- Commands echoing in terminal output
- Agent responses appearing in real-time

---

## Validation Checklist

After applying fixes, verify:

- [ ] Agent spawns without immediate termination
- [ ] Terminal shows active shell prompt (not "process terminated")
- [ ] `echo` commands work and return output
- [ ] `pwd` command shows correct directory
- [ ] Multiple commands can be sent in sequence
- [ ] TC6-TC9 all pass
- [ ] No "PTY process monitor ended" errors in logs

---

## Code Changes Required

### File: `tests/e2e/claude-terminal-interaction.spec.js`

**Line 34 - Change workspace path:**
```javascript
// BEFORE:
workspacePath: 'D:\\Code\\PythonProjects',

// AFTER:
workspacePath: 'D:\\Code\\WebProjects\\agentmux\\apps\\desktop',
```

**That's it!** This single change may fix all TC6-TC9 tests.

---

## Alternative Approach: Use Workspace Environment Variable

**Even better - make it configurable:**

```javascript
// tests/e2e/claude-terminal-interaction.spec.js
const TEST_WORKSPACE = process.env.E2E_TEST_WORKSPACE ||
                       'D:\\Code\\WebProjects\\agentmux\\apps\\desktop';

before(async () => {
  agentLabel = await spawnClaudeAgent({
    workspacePath: TEST_WORKSPACE,
    label: 'E2ETestAgent',
  });
});
```

**Run with custom workspace:**
```bash
E2E_TEST_WORKSPACE="D:\MyProject" npm run test:e2e
```

---

## Next Session TODO

1. Apply Fix 1 (change workspace path)
2. Run `npm run test:e2e`
3. Check if TC6-TC9 now pass
4. If not, review Rust logs for error messages
5. Apply additional fixes based on error output

---

## Related Files

- Test spec: `tests/e2e/claude-terminal-interaction.spec.js:34`
- Test helpers: `tests/e2e/helpers/claude-helpers.js:26-65`
- Rust spawn: `src-tauri/src/embedded_claude/process.rs:spawn_embedded_claude`
- Test config: `wdio.conf.js:77-131`

---

## Success Criteria

**E2E tests will be considered fully working when:**

1. ✅ Infrastructure tests TC1-TC5 pass (already working!)
2. ✅ TC6: Message sending test passes
3. ✅ TC7: Response reception test passes
4. ✅ TC8: Full 2-way communication test passes
5. ✅ TC9: Multiple exchanges test passes
6. ✅ No process termination errors
7. ✅ Tests run reliably (not flaky)

---

**Estimated Fix Time:** 10-15 minutes (just change workspace path and test)
**Confidence Level:** High - workspace path is very likely the root cause

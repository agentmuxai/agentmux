# AgentMux E2E Test Migration Plan

**Agent:** AgentX
**Date:** 2025-10-16
**Status:** 🔴 CRITICAL - All e2e tests broken by UI rewrite

---

## Executive Summary

The AgentMux UI underwent a complete rewrite from SimpleTerminal (DOM-based) to xterm.js (canvas-based) with WebSocket communication. **All 9 existing e2e test cases are broken** because they target the old UI structure. This document provides a comprehensive migration plan to restore test coverage and add tests for new features.

**Critical Findings:**
- ❌ All test selectors target removed UI elements
- ❌ Cannot read xterm.js terminal content via DOM (canvas-based)
- ❌ Helper functions use wrong selectors throughout
- ❌ New features have zero test coverage (panes, directory menu, layout persistence)
- ✅ Test infrastructure (WebdriverIO + tauri-driver) is sound
- ✅ Test configuration works correctly

**Estimated Effort:** 2-3 days (16-24 hours)

---

## 1. Current State Assessment

### Test Infrastructure ✅ SOLID
```
tests/
├── e2e/
│   ├── claude-terminal-interaction.spec.js  ❌ 9 broken tests
│   └── helpers/
│       ├── claude-helpers.js                ❌ All selectors wrong
│       └── tauri-app.js                     ✅ Generic helpers OK
└── wdio.conf.js                             ✅ Configuration OK
```

**Test Coverage:**
- TC1: Terminal renders and shows connection ❌
- TC2: Click output → input focused ❌
- TC3: Arrow key handling ❌
- TC4: Auto-focus behavior ❌
- TC5: Agent list display ✅ (agent cards still exist)
- TC6-TC9: Message sending and 2-way communication ❌

**Infrastructure Status:**
- ✅ WebdriverIO 8.x installed and configured
- ✅ tauri-driver integration working
- ✅ Release build automation working
- ✅ Screenshot debugging setup
- ✅ Single-instance mode disabled for testing

---

## 2. UI Architecture Changes

### Old UI (SimpleTerminal) - REMOVED
```tsx
<div class="simple-terminal">
  <div class="terminal-output">
    {messages.map(msg => <div>{msg}</div>)}  // DOM text nodes
  </div>
  <input class="terminal-input" />           // Regular input element
</div>
```

**Test Approach:** Direct DOM querying
- `.simple-terminal` → container
- `.terminal-output` → read text with `.getText()`
- `.terminal-input` → type with `.setValue()`

### New UI (EmbeddedTerminal) - CURRENT
```tsx
<div class="embedded-terminal">
  <div class="terminal-header">
    <span class="status-dot online|offline"></span>
    <span class="terminal-title">{instanceName}</span>
    <span class="terminal-port">ws://localhost:{wsPort}</span>
  </div>
  <div class="terminal-container">
    {/* xterm.js mounts here - CANVAS RENDERING */}
    {/* Content not accessible via DOM */}
  </div>
</div>
```

**Architecture:**
1. **xterm.js Terminal** - Canvas-based rendering
2. **WebSocket** - PTY output streaming (`ws://localhost:{wsPort}`)
3. **Tauri Commands** - Input via `send_claude_input`
4. **PaneContainer** - Manages multiple terminals with split-pane layout

**Test Challenges:**
- ❌ Cannot use `.getText()` on canvas
- ❌ Cannot use `.setValue()` on canvas
- ❌ Terminal content is rendered pixels, not DOM nodes
- ✅ Can test WebSocket connection state
- ✅ Can test UI components (header, status dot, panes)
- ✅ Can potentially use xterm.js accessibility buffer

---

## 3. Selector Migration Table

### Core Terminal Components

| Old Selector | Status | New Selector | Notes |
|-------------|--------|--------------|-------|
| `.simple-terminal` | ❌ REMOVED | `.embedded-terminal` | Container class changed |
| `.terminal-output` | ❌ REMOVED | N/A | xterm.js uses canvas |
| `.terminal-input` | ❌ REMOVED | N/A | xterm.js uses canvas |
| N/A | ➕ NEW | `.terminal-header` | Header with status/title/port |
| N/A | ➕ NEW | `.status-dot` | Connection indicator |
| N/A | ➕ NEW | `.status-dot.online` | Connected state |
| N/A | ➕ NEW | `.status-dot.offline` | Disconnected state |
| N/A | ➕ NEW | `.terminal-title` | Instance name display |
| N/A | ➕ NEW | `.terminal-port` | WebSocket port display |
| N/A | ➕ NEW | `.terminal-container` | xterm.js mount point |

### Agent Selection (Still Works)

| Selector | Status | Notes |
|----------|--------|-------|
| `.agent-card` | ✅ VALID | Agent selection cards |
| `.agent-card h3` | ✅ VALID | Agent name |
| `.agent-card p` | ✅ VALID | Agent description |

### Pane Management (NEW FEATURES)

| Selector | Status | Notes |
|----------|--------|-------|
| `.pane-container` | ➕ NEW | Split-pane container |
| `.pane` | ➕ NEW | Individual pane |
| `.pane-active` | ➕ NEW | Active pane indicator |
| `.pane-header` | ➕ NEW | Pane header bar |
| `.pane-title` | ➕ NEW | Pane instance name |
| `.pane-close-btn` | ➕ NEW | Close pane button |
| `.split-btn` | ➕ NEW | Split pane button |

---

## 4. Testing Strategy for xterm.js

### Challenge: Canvas-Based Rendering
xterm.js renders terminal content to HTML5 canvas, making traditional DOM-based text reading impossible.

### Approach A: UI State Testing (RECOMMENDED)
**Focus on what we CAN test:**

✅ **Connection State**
```javascript
// Test WebSocket connection
const statusDot = await $('.status-dot');
await browser.waitUntil(async () => {
  const classes = await statusDot.getAttribute('class');
  return classes.includes('online');
}, { timeout: 5000 });
```

✅ **Instance Metadata**
```javascript
// Verify instance name and port displayed
const title = await $('.terminal-title');
const titleText = await title.getText();
expect(titleText).toBe('claude-agent-1');

const port = await $('.terminal-port');
const portText = await port.getText();
expect(portText).toMatch(/ws:\/\/localhost:\d+/);
```

✅ **UI Interaction**
```javascript
// Test focus, clicks, keyboard events
const container = await $('.terminal-container');
await container.click(); // Should focus terminal
await browser.keys(['ArrowUp']); // Should send to xterm.js
```

### Approach B: Accessibility Buffer Testing (POSSIBLE)
xterm.js maintains an internal accessibility buffer for screen readers. We might be able to access it:

```javascript
// EXPERIMENTAL - needs investigation
const xtermElement = await $('.terminal-container .xterm');
const ariaLiveRegion = await xtermElement.$('[aria-live="polite"]');
const terminalContent = await ariaLiveRegion.getText();
```

**Pros:** Could read actual terminal content
**Cons:** Not officially supported, may be unreliable
**Status:** Requires research

### Approach C: Backend State Testing (RELIABLE)
Test via Tauri commands and WebSocket state instead of UI:

```javascript
// Use Tauri's test mode to expose backend state
// Add test-only command: get_instance_state(instanceName)
const state = await browser.execute(() => {
  return window.__TAURI__.invoke('get_instance_state', {
    instanceName: 'claude-agent-1'
  });
});

expect(state.isConnected).toBe(true);
expect(state.pid).toBeGreaterThan(0);
```

**Pros:** Reliable, tests actual backend state
**Cons:** Requires adding test-only Tauri commands
**Status:** RECOMMENDED for critical tests

### Recommended Hybrid Strategy
1. **UI State Tests** - Connection status, metadata, UI interactions (80% coverage)
2. **Backend State Tests** - Add test-only Tauri commands for critical state (15% coverage)
3. **Visual Testing** - Screenshot comparison for terminal appearance (5% coverage)

---

## 5. New Features Requiring Tests

### A. Pane Management (HIGH PRIORITY)

**Features:**
- Split panes vertically/horizontally
- Close individual panes
- Active pane tracking
- Layout persistence (localStorage)

**Test Cases Needed:**
```javascript
describe('Pane Management', () => {
  it('TC10: Should split pane vertically', async () => {
    await clickElement('.split-vertical-btn');
    const panes = await $$('.pane');
    expect(panes).toHaveLength(2);
  });

  it('TC11: Should split pane horizontally', async () => {
    await clickElement('.split-horizontal-btn');
    const panes = await $$('.pane');
    expect(panes).toHaveLength(2);
    const container = await $('.pane-container');
    expect(await container.getAttribute('class')).toContain('horizontal');
  });

  it('TC12: Should close pane with X button', async () => {
    await splitVertical();
    await clickElement('.pane-close-btn');
    const panes = await $$('.pane');
    expect(panes).toHaveLength(1);
  });

  it('TC13: Should mark active pane', async () => {
    await splitVertical();
    const panes = await $$('.pane');
    await panes[1].click();
    expect(await panes[1].getAttribute('class')).toContain('pane-active');
  });

  it('TC14: Should persist layout across sessions', async () => {
    await splitVertical();
    // Restart app
    await browser.reloadSession();
    await waitForAppReady();
    const panes = await $$('.pane');
    expect(panes).toHaveLength(2);
  });
});
```

### B. Working Directory Menu (HIGH PRIORITY)

**Features:**
- Right-click context menu on terminal
- Directory picker dialog
- Instance respawn with new working directory

**Test Cases Needed:**
```javascript
describe('Working Directory Menu', () => {
  it('TC15: Should show context menu on right-click', async () => {
    const terminal = await $('.terminal-container');
    await terminal.click({ button: 'right' });
    // Note: Tauri dialog may not be testable via WebDriver
    // May need to mock or use backend testing
  });

  it('TC16: Should respawn instance with new directory', async () => {
    // This requires mocking the directory picker
    // or testing via backend state
    const oldPid = await getInstancePid('claude-agent-1');
    await changeDirectory('claude-agent-1', 'D:/Code/NewProject');
    const newPid = await getInstancePid('claude-agent-1');
    expect(newPid).not.toBe(oldPid);
  });
});
```

### C. WebSocket Connection Management (MEDIUM PRIORITY)

**Features:**
- Automatic reconnection on disconnect
- Connection status indicator
- Error handling

**Test Cases Needed:**
```javascript
describe('WebSocket Connection', () => {
  it('TC17: Should show online status when connected', async () => {
    await selectAgent('Claude Code');
    const statusDot = await $('.status-dot');
    await browser.waitUntil(async () => {
      return (await statusDot.getAttribute('class')).includes('online');
    }, { timeout: 5000 });
  });

  it('TC18: Should show offline status on disconnect', async () => {
    // Kill WebSocket server or backend process
    await killInstance('claude-agent-1');
    const statusDot = await $('.status-dot');
    await browser.waitUntil(async () => {
      return (await statusDot.getAttribute('class')).includes('offline');
    }, { timeout: 5000 });
  });

  it('TC19: Should attempt reconnection after disconnect', async () => {
    await killInstance('claude-agent-1');
    await browser.pause(3000); // Wait for reconnect attempt (2s + buffer)
    // Restart backend
    await spawnInstance('claude-agent-1');
    const statusDot = await $('.status-dot');
    await browser.waitUntil(async () => {
      return (await statusDot.getAttribute('class')).includes('online');
    }, { timeout: 10000 });
  });
});
```

---

## 6. Implementation Plan

### Phase 1: Fix Existing Tests (8-10 hours)

**Step 1.1: Update Selectors in Helpers**
```javascript
// tests/e2e/helpers/claude-helpers.js

// ❌ REMOVE
export async function clickTerminalOutput() {
  const output = await getElement('.terminal-output');
  await output.click();
}

// ✅ ADD
export async function clickTerminalContainer() {
  const container = await getElement('.terminal-container');
  await container.click();
}

// ❌ REMOVE
export async function getTerminalOutput() {
  const output = await getElement('.terminal-output');
  return await output.getText();
}

// ✅ ADD (with caveat)
export async function getTerminalConnectionStatus() {
  const statusDot = await getElement('.status-dot');
  const classes = await statusDot.getAttribute('class');
  return classes.includes('online') ? 'online' : 'offline';
}

export async function getTerminalInstanceName() {
  const title = await getElement('.terminal-title');
  return await title.getText();
}

export async function getTerminalPort() {
  const port = await getElement('.terminal-port');
  const text = await port.getText();
  return text.match(/ws:\/\/localhost:(\d+)/)?.[1];
}
```

**Step 1.2: Rewrite TC1-TC4 (Terminal Rendering & Focus)**
```javascript
// TC1: Terminal renders and shows connection
it('TC1: Terminal renders and shows connection', async () => {
  await selectAgent('Claude Code');

  // Wait for terminal to render
  const terminal = await getElement('.embedded-terminal', 10000);
  expect(await terminal.isDisplayed()).toBe(true);

  // Verify header elements
  const header = await terminal.$('.terminal-header');
  expect(await header.isDisplayed()).toBe(true);

  // Verify connection status
  const status = await getTerminalConnectionStatus();
  expect(status).toBe('online');

  // Verify instance name
  const instanceName = await getTerminalInstanceName();
  expect(instanceName).toMatch(/claude-agent-\d+/);

  // Verify WebSocket port
  const port = await getTerminalPort();
  expect(parseInt(port)).toBeGreaterThan(9000);
  expect(parseInt(port)).toBeLessThan(10000);

  await takeDebugScreenshot('TC1-terminal-connected');
});

// TC2: Click container focuses terminal (xterm.js internal focus)
it('TC2: Click terminal container triggers focus', async () => {
  await selectAgent('Claude Code');
  const container = await getElement('.terminal-container');

  // Click container
  await container.click();

  // Verify active state (if panes exist)
  const pane = await container.$('..').$('.pane'); // parent pane
  if (await pane.isExisting()) {
    const classes = await pane.getAttribute('class');
    expect(classes).toContain('pane-active');
  }

  // xterm.js internal focus is hard to test - verify no errors
  await takeDebugScreenshot('TC2-terminal-clicked');
});

// TC3: Arrow key handling (can test key dispatch, not content)
it('TC3: Arrow keys are dispatched to terminal', async () => {
  await selectAgent('Claude Code');
  const container = await getElement('.terminal-container');
  await container.click();

  // Send arrow key - xterm.js will handle internally
  await browser.keys(['ArrowUp']);
  await browser.keys(['ArrowDown']);

  // Cannot verify output, but verify no crashes
  const status = await getTerminalConnectionStatus();
  expect(status).toBe('online');

  await takeDebugScreenshot('TC3-arrow-keys-sent');
});

// TC4: Auto-focus (first terminal gets focus)
it('TC4: First terminal gets focus automatically', async () => {
  await selectAgent('Claude Code');
  await browser.pause(1000); // Wait for auto-focus

  // Verify pane is active
  const pane = await getElement('.pane');
  const classes = await pane.getAttribute('class');
  expect(classes).toContain('pane-active');

  await takeDebugScreenshot('TC4-auto-focus');
});
```

**Step 1.3: Update TC5 (Agent List) - MINOR CHANGES**
```javascript
// TC5: Agent list display - mostly works, update selectors
it('TC5: Agent list displays correctly', async () => {
  const agentCards = await $$('.agent-card');
  expect(agentCards.length).toBeGreaterThan(0);

  const firstCard = agentCards[0];
  const name = await firstCard.$('h3').getText();
  expect(name).toBeTruthy();

  // Click to spawn
  await firstCard.click();

  // Wait for terminal to appear (updated selector)
  await getElement('.embedded-terminal', 10000);

  await takeDebugScreenshot('TC5-agent-selected');
});
```

**Step 1.4: Rewrite TC6-TC9 (Message Sending) - BACKEND TESTING**
```javascript
// TC6-TC9: Message sending requires backend state testing
// Add test-only Tauri command: get_instance_output(instanceName, lastN)

// First, add to src-tauri/src/tauri_commands/test_helpers.rs:
/*
#[tauri::command]
#[cfg(test)]
pub async fn get_instance_output(
    instance_name: String,
    last_n: usize,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    // Return last N lines from instance buffer
    // Implementation needed
}
*/

// Then test:
it('TC6: Can send message to Claude instance', async () => {
  await selectAgent('Claude Code');
  const terminal = await getElement('.embedded-terminal');
  await terminal.$('.terminal-container').click();

  // Send via keyboard
  await browser.keys('hello world');
  await browser.keys(['Enter']);

  // Verify via backend (if test command added)
  // OR verify connection stays alive
  await browser.pause(500);
  const status = await getTerminalConnectionStatus();
  expect(status).toBe('online');

  await takeDebugScreenshot('TC6-message-sent');
});

// TC7-TC9: Similar pattern - focus on connection state, not content
```

**Estimated Time:** 8-10 hours
- Selector updates: 2 hours
- TC1-TC4 rewrite: 3 hours
- TC5 updates: 1 hour
- TC6-TC9 rewrite: 3 hours
- Testing/debugging: 2 hours

---

### Phase 2: Add New Feature Tests (6-8 hours)

**Step 2.1: Pane Management Tests (TC10-TC14)**
- Add split pane tests: 2 hours
- Add close pane tests: 1 hour
- Add active pane tests: 1 hour
- Add layout persistence tests: 2 hours

**Step 2.2: Working Directory Menu Tests (TC15-TC16)**
- Research Tauri dialog testing: 1 hour
- Implement directory change tests: 2 hours

**Step 2.3: Connection Management Tests (TC17-TC19)**
- Connection status tests: 1 hour
- Reconnection tests: 2 hours

**Estimated Time:** 6-8 hours

---

### Phase 3: Test Infrastructure Improvements (2-4 hours)

**Step 3.1: Add Backend Test Helpers**
```rust
// src-tauri/src/tauri_commands/test_helpers.rs (NEW FILE)

#[cfg(test)]
use tauri::State;
use crate::tauri_commands::types::AppState;

#[tauri::command]
#[cfg(feature = "test-helpers")]
pub async fn get_instance_state(
    instance_name: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let instances = state.claude_instances.lock().await;
    let instance = instances.get(&instance_name)
        .ok_or_else(|| format!("Instance '{}' not found", instance_name))?;

    Ok(serde_json::json!({
        "pid": instance.pid,
        "ws_port": instance.ws_port,
        "is_connected": true, // Add connection tracking
    }))
}

// More test helpers...
```

**Step 3.2: Improve Screenshot Debugging**
```javascript
// tests/e2e/helpers/tauri-app.js

export async function takeDebugScreenshot(name, options = {}) {
  const timestamp = Date.now();
  const filename = `${name}-${timestamp}.png`;

  await browser.saveScreenshot(`./test-results/screenshots/${filename}`);

  if (options.logHTML) {
    const html = await browser.execute(() => document.body.outerHTML);
    await fs.writeFile(
      `./test-results/html/${name}-${timestamp}.html`,
      html
    );
  }

  console.log(`[Tauri E2E] ✓ Screenshot: ${filename}`);
}
```

**Step 3.3: Add Visual Regression Testing (Optional)**
```javascript
// Use wdio-image-comparison-service
import { join } from 'path';

export async function compareTerminalScreenshot(name) {
  const terminal = await $('.embedded-terminal');
  await terminal.saveElement(join('baseline', `${name}.png`));

  const result = await browser.checkElement(terminal, name);
  expect(result).toBeLessThan(5); // 5% difference threshold
}
```

**Estimated Time:** 2-4 hours

---

### Phase 4: Documentation & Cleanup (1-2 hours)

**Step 4.1: Update Test Documentation**
```markdown
# tests/README.md

## Running E2E Tests

### Prerequisites
- tauri-driver installed
- msedgedriver.exe in project root
- AgentMux built in release mode

### Commands
npm run test:e2e              # Run all e2e tests
npm run test:e2e:spec -- TC1  # Run specific test

### Test Coverage
- TC1-TC4: Terminal rendering & focus
- TC5: Agent selection
- TC6-TC9: Message sending
- TC10-TC14: Pane management
- TC15-TC16: Working directory menu
- TC17-TC19: Connection management

### Debugging
- Screenshots saved to test-results/screenshots/
- Use takeDebugScreenshot(name) in tests
- Enable WDIO verbose logging: logLevel: 'debug'
```

**Step 4.2: Clean Up Old Code**
- Remove obsolete helper functions
- Remove commented-out code
- Update inline documentation

**Estimated Time:** 1-2 hours

---

## 7. Risk Assessment

### High Risks 🔴

1. **xterm.js Content Reading**
   - **Risk:** Cannot verify actual terminal output
   - **Mitigation:** Focus on connection state, add backend test commands
   - **Fallback:** Visual regression testing with screenshots

2. **WebSocket Timing Issues**
   - **Risk:** Connection takes time, race conditions in tests
   - **Mitigation:** Use proper `waitUntil` with polling, increase timeouts
   - **Fallback:** Add retry logic

3. **Tauri Dialog Testing**
   - **Risk:** Native dialogs may not be accessible via WebDriver
   - **Mitigation:** Mock dialog responses, test via backend state
   - **Fallback:** Manual testing for directory picker

### Medium Risks 🟡

4. **Layout Persistence Testing**
   - **Risk:** localStorage may persist between test runs
   - **Mitigation:** Clear localStorage in `beforeEach` hook
   - **Fallback:** Use unique keys per test

5. **Multi-Instance Conflicts**
   - **Risk:** Multiple test instances may conflict
   - **Mitigation:** `AGENTMUX_DISABLE_SINGLE_INSTANCE` already set
   - **Fallback:** Run tests sequentially (already configured)

6. **Release Build Slowness**
   - **Risk:** Building release mode before each test run is slow
   - **Mitigation:** Cache build, only rebuild on code changes
   - **Fallback:** Accept slower test runs

### Low Risks 🟢

7. **Test Infrastructure Stability**
   - **Risk:** WebdriverIO or tauri-driver breaking changes
   - **Mitigation:** Lock dependency versions
   - **Fallback:** Well-documented, easy to update

---

## 8. Timeline Estimate

### Optimistic (16 hours)
- Phase 1: 8 hours
- Phase 2: 6 hours
- Phase 3: 2 hours
- Phase 4: 1 hour

### Realistic (24 hours)
- Phase 1: 10 hours (includes debugging)
- Phase 2: 8 hours (includes research)
- Phase 3: 4 hours (includes backend work)
- Phase 4: 2 hours

### Pessimistic (32+ hours)
- Phase 1: 12 hours (xterm.js issues)
- Phase 2: 10 hours (Tauri dialog challenges)
- Phase 3: 6 hours (complex backend changes)
- Phase 4: 2 hours
- Phase 5: 2+ hours (unforeseen issues)

**Recommended:** Plan for 3 full days (24 hours) with buffer

---

## 9. Success Criteria

### Must Have ✅
- [ ] All 9 existing tests passing with updated selectors
- [ ] TC10-TC14: Full pane management coverage
- [ ] TC17-TC19: Connection management tests
- [ ] Zero failures in CI pipeline
- [ ] Screenshot debugging functional

### Should Have ⭐
- [ ] TC15-TC16: Working directory menu tests
- [ ] Backend test helpers for state verification
- [ ] Visual regression baseline for terminal rendering
- [ ] Test execution time < 5 minutes

### Nice to Have 🎯
- [ ] Accessibility buffer testing for terminal content
- [ ] Performance benchmarks for terminal rendering
- [ ] Stress tests for multiple panes
- [ ] E2E tests for bus messaging features

---

## 10. Next Steps

### Immediate Actions (Today)
1. ✅ Review and approve this plan
2. Create feature branch: `test/e2e-migration`
3. Start Phase 1, Step 1.1: Update selectors in helpers

### This Week
1. Complete Phase 1: Fix existing tests (Days 1-2)
2. Complete Phase 2: Add new feature tests (Day 3)

### Next Week
1. Complete Phase 3: Infrastructure improvements
2. Complete Phase 4: Documentation
3. Run full test suite and verify all passing
4. Create PR with comprehensive test results

---

## Appendices

### A. Key Files to Modify

```
tests/
├── e2e/
│   ├── claude-terminal-interaction.spec.js  [MAJOR REWRITE]
│   └── helpers/
│       ├── claude-helpers.js                [MAJOR REWRITE]
│       └── tauri-app.js                     [MINOR UPDATES]
├── README.md                                 [UPDATE]
└── wdio.conf.js                             [MINOR UPDATES]

src-tauri/src/
└── tauri_commands/
    └── test_helpers.rs                      [NEW FILE]
```

### B. Useful References

- xterm.js API: https://xtermjs.org/docs/api/terminal/
- WebdriverIO Docs: https://webdriver.io/docs/api
- Tauri Testing: https://tauri.app/v1/guides/testing/
- Accessibility Buffer: https://github.com/xtermjs/xterm.js/issues/2564

### C. Example Test Run Output (Expected)

```bash
$ npm run test:e2e

> agentmux@0.3.28 test:e2e
> wdio run wdio.conf.js

[wdio] Building frontend...
[wdio] ✓ Frontend built
[wdio] Building Tauri app (release mode)...
[wdio] ✓ Tauri app built
[wdio] Starting tauri-driver server...
[wdio] ✓ tauri-driver server started

Execution of 1 spec files started at 2025-10-16T12:00:00.000Z

[chrome 110.0.5481.104 windows #0-0] Running: chrome (v110.0.5481.104) on windows
[chrome 110.0.5481.104 windows #0-0] Session ID: abc123def456
[chrome 110.0.5481.104 windows #0-0]
[chrome 110.0.5481.104 windows #0-0] » /tests/e2e/claude-terminal-interaction.spec.js
[chrome 110.0.5481.104 windows #0-0] AgentMux - Claude Terminal Interaction
[chrome 110.0.5481.104 windows #0-0]    ✓ TC1: Terminal renders and shows connection
[chrome 110.0.5481.104 windows #0-0]    ✓ TC2: Click terminal container triggers focus
[chrome 110.0.5481.104 windows #0-0]    ✓ TC3: Arrow keys are dispatched to terminal
[chrome 110.0.5481.104 windows #0-0]    ✓ TC4: First terminal gets focus automatically
[chrome 110.0.5481.104 windows #0-0]    ✓ TC5: Agent list displays correctly
[chrome 110.0.5481.104 windows #0-0]    ✓ TC6: Can send message to Claude instance
[chrome 110.0.5481.104 windows #0-0]    ✓ TC7: Receives response from Claude
[chrome 110.0.5481.104 windows #0-0]    ✓ TC8: Can send multiple messages
[chrome 110.0.5481.104 windows #0-0]    ✓ TC9: Handles special characters
[chrome 110.0.5481.104 windows #0-0]
[chrome 110.0.5481.104 windows #0-0] AgentMux - Pane Management
[chrome 110.0.5481.104 windows #0-0]    ✓ TC10: Should split pane vertically
[chrome 110.0.5481.104 windows #0-0]    ✓ TC11: Should split pane horizontally
[chrome 110.0.5481.104 windows #0-0]    ✓ TC12: Should close pane with X button
[chrome 110.0.5481.104 windows #0-0]    ✓ TC13: Should mark active pane
[chrome 110.0.5481.104 windows #0-0]    ✓ TC14: Should persist layout across sessions
[chrome 110.0.5481.104 windows #0-0]
[chrome 110.0.5481.104 windows #0-0] AgentMux - Connection Management
[chrome 110.0.5481.104 windows #0-0]    ✓ TC17: Should show online status when connected
[chrome 110.0.5481.104 windows #0-0]    ✓ TC18: Should show offline status on disconnect
[chrome 110.0.5481.104 windows #0-0]    ✓ TC19: Should attempt reconnection after disconnect
[chrome 110.0.5481.104 windows #0-0]
[chrome 110.0.5481.104 windows #0-0] 17 passing (45.2s)

[wdio] Stopping tauri-driver server...
[wdio] ✓ tauri-driver server stopped

Spec Files:      1 passed, 1 total (100% completed) in 00:00:48
```

---

**End of Plan**

Ready to proceed with Phase 1?

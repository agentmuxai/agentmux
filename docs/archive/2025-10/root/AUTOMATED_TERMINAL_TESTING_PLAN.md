# Automated Terminal Testing Plan for AgentMux

**Date:** 2025-10-15
**Version:** 1.0
**Purpose:** Automated E2E testing of Claude terminal interactions with focus capture and response verification

---

## Executive Summary

This plan outlines how to automate testing of the Claude terminal UI, including:
1. **Focus unification** - Click anywhere on terminal area (output OR input) → input gets focus
2. **Keyboard event handling** - Arrow keys navigate Claude menus, Enter confirms selections
3. **Response verification** - Send input and verify Claude responds with expected output

## Current Testing Infrastructure

### ✅ What We Already Have

1. **Playwright E2E Framework**
   - Location: `apps/desktop/tests/e2e/`
   - Config: `apps/desktop/playwright.config.ts`
   - Helper utilities: `tests/e2e/helpers/tauri-app.ts`

2. **Tauri App Launch Mechanism**
   - Uses Chrome DevTools Protocol (CDP) to connect to WebView2
   - Launches app with remote debugging: `--remote-debugging-port=9222`
   - Helper function: `launchTauriApp()` returns browser + page + process

3. **Existing Test Examples**
   - `agents-manager.spec.ts` - Spawn agents, send input, verify output
   - `message-stream.spec.ts` - Message flow testing
   - Helper: `takeDebugScreenshot()` for visual debugging

### ✅ Key Technologies

- **Playwright** - Browser automation (already installed)
- **Tauri WebView2** - Native Windows WebView with CDP support
- **Portable PTY** - Pseudoterminal for Claude subprocess
- **WebSocket** - PTY stdout/stdin forwarding to UI

---

## Problem Statement

### Current Issues

1. **Separate Focus Contexts**
   - Terminal output (read-only div) has its own scrollable area
   - Input field is separate element
   - User expectation: Click anywhere → can type immediately

2. **Keyboard Event Leakage**
   - Arrow keys scroll the output div instead of sending to Claude
   - Need `stopPropagation()` to prevent bubbling (already added in v0.3.16)

3. **Manual Testing Required**
   - Currently testing by manually launching app
   - No automated verification of Claude responses

---

## Solution Architecture

### Phase 1: Focus Unification (UI Fix)

**Goal:** Make terminal behave as single interactive unit

**Implementation:**
```typescript
// SimpleTerminal.tsx
const terminalContainerRef = createSignal<HTMLDivElement>();

const focusInput = () => {
  const input = document.querySelector('.terminal-input') as HTMLInputElement;
  input?.focus();
};

return (
  <div
    class="simple-terminal"
    ref={terminalContainerRef}
    onClick={focusInput}  // Click anywhere → focus input
    tabIndex={-1}          // Make container focusable but not tab-stoppable
  >
    <div class="terminal-output" innerHTML={...} />
    <input
      class="terminal-input"
      onKeyDown={handleKeyDown}
      autoFocus  // Auto-focus on mount
    />
  </div>
);
```

**Why This Works:**
- Container captures all clicks (output or input area)
- Delegates focus to input field
- Input immediately ready for keyboard events

### Phase 2: Automated Testing (E2E)

**Test File:** `apps/desktop/tests/e2e/claude-terminal-interaction.spec.ts`

#### Test Case 1: Focus Capture

```typescript
test('Terminal focuses input when output area clicked', async () => {
  const { page } = await launchTauriApp();

  // Spawn Claude instance
  await page.click('button:has-text("Spawn Agent")');
  await page.fill('input[placeholder*="agent"]', 'TestClaude');
  await page.click('button:has-text("Spawn")');
  await page.waitForTimeout(2000);

  // Click on terminal OUTPUT area (not input)
  const terminalOutput = page.locator('.terminal-output').first();
  await terminalOutput.click();

  // Verify input field is now focused
  const inputField = page.locator('.terminal-input').first();
  await expect(inputField).toBeFocused();

  console.log('[Test] ✓ Terminal output click → input focused');
});
```

#### Test Case 2: Keyboard Event Handling

```typescript
test('Arrow keys navigate Claude menu without scrolling', async () => {
  const { page } = await launchTauriApp();

  // Spawn Claude and wait for trust prompt
  await spawnClaudeAgent(page);
  await page.waitForSelector('text=/Do you trust/i', { timeout: 10000 });

  // Verify menu is visible
  const menu = page.locator('text=/Yes, proceed/i');
  await expect(menu).toBeVisible();

  // Click terminal to focus
  await page.click('.simple-terminal');

  // Get initial scroll position
  const outputDiv = page.locator('.terminal-output');
  const initialScroll = await outputDiv.evaluate(el => el.scrollTop);

  // Press arrow down
  await page.keyboard.press('ArrowDown');
  await page.waitForTimeout(100);

  // Verify scroll position UNCHANGED (event didn't bubble)
  const afterScroll = await outputDiv.evaluate(el => el.scrollTop);
  expect(afterScroll).toBe(initialScroll);

  console.log('[Test] ✓ Arrow key didn\'t scroll output');
});
```

#### Test Case 3: Claude Response Verification

```typescript
test('Claude responds to Enter key confirmation', async () => {
  const { page } = await launchTauriApp();

  // Spawn Claude instance
  await spawnClaudeAgent(page, 'D:\\Code\\PythonProjects');

  // Wait for trust prompt
  await page.waitForSelector('text=/Do you trust/i', { timeout: 10000 });

  // Take screenshot of prompt
  await takeDebugScreenshot(page, 'claude-01-trust-prompt');

  // Press Enter to confirm (input should be empty)
  const inputField = page.locator('.terminal-input');
  await inputField.click();
  await inputField.fill('');  // Ensure empty
  await page.keyboard.press('Enter');

  console.log('[Test] Sent Enter key to Claude');

  // Wait for Claude to respond (trust prompt should disappear)
  await page.waitForSelector('text=/Do you trust/i', {
    state: 'hidden',
    timeout: 5000
  });

  // Verify Claude prompt appears
  await page.waitForSelector('text=/Claude Code/i', { timeout: 5000 });

  await takeDebugScreenshot(page, 'claude-02-after-enter');

  console.log('[Test] ✓ Claude accepted trust and showed prompt');
});
```

#### Test Case 4: Text Input and Response

```typescript
test('Claude responds to user input', async () => {
  const { page } = await launchTauriApp();

  // Spawn Claude and get past trust prompt
  await spawnClaudeAgent(page);
  await confirmClaudeTrust(page);

  // Wait for Claude to be ready
  await page.waitForSelector('text=/What would you like/i', { timeout: 10000 });

  // Send a simple command
  const inputField = page.locator('.terminal-input');
  await inputField.fill('pwd');
  await page.keyboard.press('Enter');

  console.log('[Test] Sent command: pwd');

  // Wait for response in terminal output
  const terminalOutput = page.locator('.terminal-output');

  // Should see the command being executed
  await expect(terminalOutput).toContainText('pwd', { timeout: 5000 });

  // Should see the working directory in output
  await expect(terminalOutput).toContainText('D:\\', { timeout: 10000 });

  await takeDebugScreenshot(page, 'claude-03-pwd-response');

  console.log('[Test] ✓ Claude executed command and returned output');
});
```

### Phase 3: Helper Functions

Create reusable test utilities:

```typescript
// tests/e2e/helpers/claude-helpers.ts

export async function spawnClaudeAgent(
  page: Page,
  workdir: string = 'D:\\Code\\PythonProjects'
): Promise<void> {
  // Navigate to Agents tab
  await page.click('button:has-text("Agents")');

  // Fill spawn form
  const agentId = `claude-test-${Date.now()}`;
  await page.fill('input[placeholder*="agent"]', agentId);
  await page.fill('input[placeholder*="directory"]', workdir);

  // Click spawn
  await page.click('button:has-text("Spawn")');

  // Wait for agent to appear in list
  await page.waitForTimeout(2000);

  console.log(`[Helper] ✓ Spawned Claude agent: ${agentId}`);
}

export async function confirmClaudeTrust(page: Page): Promise<void> {
  // Wait for trust prompt
  await page.waitForSelector('text=/Do you trust/i', { timeout: 10000 });

  // Focus input and press Enter (option 1 is pre-selected)
  const inputField = page.locator('.terminal-input');
  await inputField.click();
  await inputField.fill('');
  await page.keyboard.press('Enter');

  // Wait for prompt to disappear
  await page.waitForSelector('text=/Do you trust/i', { state: 'hidden', timeout: 5000 });

  console.log('[Helper] ✓ Confirmed Claude trust');
}

export async function sendClaudeCommand(
  page: Page,
  command: string
): Promise<string> {
  const inputField = page.locator('.terminal-input');

  // Clear and type command
  await inputField.fill(command);
  await page.keyboard.press('Enter');

  // Wait a bit for response
  await page.waitForTimeout(1000);

  // Return terminal output
  const output = await page.locator('.terminal-output').textContent();

  console.log(`[Helper] ✓ Sent command: ${command}`);

  return output || '';
}

export async function waitForClaudeOutput(
  page: Page,
  expectedText: string,
  timeout: number = 10000
): Promise<void> {
  const terminalOutput = page.locator('.terminal-output');
  await expect(terminalOutput).toContainText(expectedText, { timeout });

  console.log(`[Helper] ✓ Found expected output: ${expectedText}`);
}
```

---

## Implementation Roadmap

### Week 1: UI Focus Unification

**Tasks:**
1. ✅ Add `stopPropagation()` to keyboard handlers (DONE in v0.3.16)
2. Add `onClick={focusInput}` to terminal container
3. Add `autoFocus` to input field
4. Test manually: Click output → can type immediately

**Acceptance Criteria:**
- Clicking anywhere in terminal immediately allows typing
- No separate focus states between output and input

### Week 2: Basic E2E Test Setup

**Tasks:**
1. Create `claude-terminal-interaction.spec.ts`
2. Implement focus capture test
3. Implement keyboard event test (arrow keys don't scroll)
4. Run tests locally and verify

**Acceptance Criteria:**
- Tests launch app successfully
- Tests can spawn Claude instance
- Tests verify focus and keyboard behavior

### Week 3: Claude Response Testing

**Tasks:**
1. Implement trust prompt confirmation test
2. Implement command execution test
3. Create helper functions for common operations
4. Add response verification with timeouts

**Acceptance Criteria:**
- Tests can interact with Claude's interactive menu
- Tests can send commands and verify responses
- Helper functions reduce test boilerplate

### Week 4: CI Integration

**Tasks:**
1. Configure GitHub Actions workflow
2. Build app before tests
3. Run tests in headless mode
4. Upload screenshots and videos as artifacts

**Acceptance Criteria:**
- Tests run automatically on PR
- Failures are clearly reported
- Debug artifacts available for investigation

---

## Testing Strategy

### Levels of Testing

1. **Unit Tests (Vitest)** - Component-level logic
   - `SimpleTerminal.tsx` component
   - Keyboard event handlers
   - Focus management

2. **Integration Tests (Playwright)** - UI interactions
   - Terminal click → input focus
   - Keyboard events → PTY forwarding
   - WebSocket message flow

3. **E2E Tests (Playwright + Real Claude)** - End-to-end scenarios
   - Spawn Claude → Trust prompt → Command execution
   - Full user workflows

### Test Environment

**Local Development:**
```bash
# Run Playwright tests
npm run test:playwright

# Run with UI mode (visual debugging)
npm run test:playwright:ui

# Run specific test file
npx playwright test tests/e2e/claude-terminal-interaction.spec.ts
```

**CI/CD (GitHub Actions):**
```yaml
name: E2E Tests

on: [pull_request]

jobs:
  test:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v3
      - uses: actions/setup-node@v3
      - run: npm ci
      - run: npm run tauri:build
      - run: npm run test:playwright
      - uses: actions/upload-artifact@v3
        if: failure()
        with:
          name: test-results
          path: test-results/
```

---

## Success Metrics

### Functional Metrics

- ✅ Click terminal output → input focused (100% success rate)
- ✅ Arrow keys navigate menus without scrolling (0% scroll events)
- ✅ Enter key confirms Claude prompts (< 5s response time)
- ✅ Commands execute and return output (100% success rate)

### Performance Metrics

- App launch time: < 5 seconds
- Test execution time: < 60 seconds per test
- Screenshot capture: < 500ms per screenshot

### Reliability Metrics

- Test flakiness: < 5% failure rate
- CI pass rate: > 95%
- Debug artifact generation: 100% on failure

---

## Risks and Mitigations

### Risk 1: PTY Timing Issues

**Problem:** Claude may not respond within timeout
**Mitigation:**
- Use generous timeouts (10s for first response)
- Retry logic in CI (2 retries)
- Check PTY logs for errors

### Risk 2: WebView2 CDP Instability

**Problem:** Playwright connection may fail
**Mitigation:**
- Verify debugging port before connecting
- Add connection retries
- Log all CDP errors

### Risk 3: Claude Version Changes

**Problem:** Claude's interactive prompts may change
**Mitigation:**
- Use flexible selectors (regex patterns)
- Test against multiple Claude versions
- Update tests when Claude updates

---

## Next Steps

### Immediate (This Week)

1. **Fix focus unification** - Add `onClick` handler to terminal container
2. **Create basic E2E test** - Spawn Claude and verify focus behavior
3. **Run test locally** - Verify Playwright can launch app and interact

### Short Term (Next 2 Weeks)

1. **Implement full test suite** - All test cases from Phase 2
2. **Create helper functions** - Reduce test boilerplate
3. **Add CI workflow** - Automated testing on PRs

### Long Term (Next Month)

1. **Expand test coverage** - Message bus, agent communication
2. **Performance testing** - Load testing with multiple agents
3. **Visual regression testing** - Screenshot comparison

---

## Conclusion

This plan provides a comprehensive approach to automated testing of the Claude terminal UI. By leveraging existing Playwright infrastructure and implementing focus unification, we can achieve reliable, automated verification of terminal interactions.

**Key Benefits:**
- ✅ Faster iteration (automated vs manual testing)
- ✅ Early bug detection (CI runs on every PR)
- ✅ Regression prevention (tests catch breaking changes)
- ✅ Better UX (unified focus, no keyboard event leakage)

**Timeline:** 4 weeks to full implementation and CI integration

---

**Author:** AgentX
**Status:** Ready for Implementation
**Next Action:** Implement Phase 1 (Focus Unification)

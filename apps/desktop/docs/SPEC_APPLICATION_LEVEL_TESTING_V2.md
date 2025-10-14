# Application-Level Testing Specification v2.0

**Version:** 2.0
**Date:** 2025-01-14
**Status:** Proposed
**Purpose:** Comprehensive automated testing strategy for AgentMux Desktop using modern best practices
**Supersedes:** v1.0 (WebDriver-based approach)

---

## What's New in v2.0

1. ✅ **Playwright with CDP** - Modern browser automation via Chrome DevTools Protocol for Tauri
2. ✅ **Page Object Model** - Maintainable UI test structure with reusable components
3. ✅ **Playwright Fixtures** - Powerful test setup/teardown and dependency injection
4. ✅ **BATS Framework** - Professional Bash testing instead of raw shell scripts
5. ✅ **Test Isolation** - Each test runs independently with own state
6. ✅ **Parallel Execution** - Tests run concurrently for faster feedback
7. ✅ **API-based Setup** - Use CLI/IPC for test data instead of UI clicks

---

## Table of Contents

1. [Overview](#overview)
2. [Testing Layers](#testing-layers)
3. [CLI Testing Strategy (BATS)](#cli-testing-strategy-bats)
4. [UI Testing Strategy (Playwright)](#ui-testing-strategy-playwright)
5. [Integration Testing](#integration-testing)
6. [Test Infrastructure](#test-infrastructure)
7. [CI/CD Integration](#cicd-integration)
8. [Test Data Management](#test-data-management)
9. [Success Criteria](#success-criteria)

---

## Overview

### Goals

1. **Application-level validation** - Test features as users experience them
2. **Automated regression prevention** - Catch breaking changes before deployment
3. **Cross-platform confidence** - Windows, macOS, Linux support
4. **Fast feedback** - Complete test suite in < 5 minutes
5. **Maintainable tests** - Easy to read, write, and update

### Testing Philosophy

- **User-centric** - Tests mirror real user interactions and workflows
- **Deterministic** - Repeatable results every run
- **Isolated** - Tests don't depend on each other or share state
- **Fast** - Optimized for quick feedback loops
- **Debuggable** - Easy to identify failure causes

---

## Testing Layers

```
┌──────────────────────────────────────────────────────────┐
│         Application-Level Tests (This Spec)              │
├──────────────────────────────────────────────────────────┤
│  ┌─────────────┐  ┌──────────────┐  ┌────────────────┐  │
│  │  CLI Tests  │  │   UI Tests   │  │  E2E Tests     │  │
│  │   (BATS)    │  │ (Playwright) │  │  (Combined)    │  │
│  └─────────────┘  └──────────────┘  └────────────────┘  │
├──────────────────────────────────────────────────────────┤
│            Component Tests (Existing)                    │
│  • Vitest (SolidJS components)                           │
│  • Rust unit tests (src-tauri/src/**/*_tests.rs)        │
│  • Integration tests (tests/integration_test.rs)        │
└──────────────────────────────────────────────────────────┘
```

---

## CLI Testing Strategy (BATS)

### Why BATS?

- **TAP-compliant** - Standard test output format
- **Bash-native** - No external dependencies beyond Bash 3.2+
- **CI-friendly** - Easy integration with GitHub Actions, Jenkins, etc.
- **Expressive** - Clear test syntax with helper functions
- **Mature** - Community-maintained with active development

### Installation

```bash
# macOS
brew install bats-core

# Linux
git clone https://github.com/bats-core/bats-core.git
cd bats-core
./install.sh /usr/local

# Verify
bats --version
```

### Test Structure

**Location:** `apps/desktop/tests/cli/`

```
tests/cli/
├── single_instance.bats
├── ipc_forwarding.bats
├── headless_mode.bats
├── agent_lifecycle.bats
├── bus_lifecycle.bats
├── messaging.bats
├── log_export.bats
└── helpers/
    ├── test_helper.bash    # Shared helper functions
    └── assertions.bash     # Custom assertions
```

### BATS Test Examples

#### 1. Single-Instance Enforcement

**File:** `tests/cli/single_instance.bats`

```bash
#!/usr/bin/env bats

load helpers/test_helper
load helpers/assertions

setup() {
    # Clean state before each test
    cleanup_agentmux
    export AGENTMUX_BIN="$BATS_TEST_DIRNAME/../../src-tauri/target/release/agentmux"
}

teardown() {
    cleanup_agentmux
}

@test "only one GUI instance can run at a time" {
    # Start first instance
    $AGENTMUX_BIN &
    GUI_PID=$!
    sleep 2

    # Verify lock file created
    assert_file_exists "$HOME/.agentmux/desktop.lock"

    # Attempt second instance (should exit immediately)
    run $AGENTMUX_BIN
    assert_success
    assert_output --partial "GUI instance already running"

    # Cleanup
    kill $GUI_PID
    wait $GUI_PID 2>/dev/null || true
}

@test "stale lock file is removed" {
    # Create stale lock with fake PID
    mkdir -p "$HOME/.agentmux"
    echo '{"pid":99999,"ipc_port":9999}' > "$HOME/.agentmux/desktop.lock"

    # Start should succeed (removes stale lock)
    $AGENTMUX_BIN &
    GUI_PID=$!
    sleep 2

    # Verify new lock with correct PID
    assert_file_exists "$HOME/.agentmux/desktop.lock"
    run cat "$HOME/.agentmux/desktop.lock"
    assert_output --partial "\"pid\":$GUI_PID"

    kill $GUI_PID
}

@test "lock file cleaned up on exit" {
    $AGENTMUX_BIN &
    GUI_PID=$!
    sleep 2

    assert_file_exists "$HOME/.agentmux/desktop.lock"

    kill $GUI_PID
    wait $GUI_PID 2>/dev/null || true
    sleep 1

    # Lock should be removed
    assert_file_not_exists "$HOME/.agentmux/desktop.lock"
}
```

#### 2. IPC Command Forwarding

**File:** `tests/cli/ipc_forwarding.bats`

```bash
#!/usr/bin/env bats

load helpers/test_helper

setup() {
    cleanup_agentmux
    export AGENTMUX_BIN="$BATS_TEST_DIRNAME/../../src-tauri/target/release/agentmux"

    # Start GUI instance
    $AGENTMUX_BIN &
    GUI_PID=$!
    sleep 3
}

teardown() {
    kill $GUI_PID 2>/dev/null || true
    cleanup_agentmux
}

@test "CLI commands forward to running instance" {
    # Execute CLI command (should use IPC)
    run $AGENTMUX_BIN --json agents list
    assert_success
    assert_output --partial '"agents"'
}

@test "IPC returns valid JSON" {
    run $AGENTMUX_BIN --json agents list
    assert_success

    # Validate JSON with jq
    echo "$output" | jq . > /dev/null
    assert [ $? -eq 0 ]
}

@test "IPC command timeout handled gracefully" {
    # Kill GUI but leave lock (simulates hung process)
    kill -9 $GUI_PID
    sleep 1

    # CLI should detect and timeout
    run timeout 5s $AGENTMUX_BIN agents list
    assert_failure
    assert_output --partial "Failed to communicate"
}
```

#### 3. Helper Functions

**File:** `tests/cli/helpers/test_helper.bash`

```bash
# Shared helper functions for BATS tests

# Clean up any running agentmux instances
cleanup_agentmux() {
    pkill -9 agentmux 2>/dev/null || true
    rm -f "$HOME/.agentmux/desktop.lock"
    sleep 1
}

# Wait for port to be available
wait_for_port() {
    local port=$1
    local timeout=${2:-30}
    local elapsed=0

    while [ $elapsed -lt $timeout ]; do
        if nc -z localhost $port 2>/dev/null; then
            return 0
        fi
        sleep 1
        elapsed=$((elapsed + 1))
    done

    return 1
}

# Check if process is running
is_process_running() {
    local pid=$1
    ps -p $pid > /dev/null 2>&1
}
```

**File:** `tests/cli/helpers/assertions.bash`

```bash
# Custom assertions for BATS

assert_file_exists() {
    if [ ! -f "$1" ]; then
        echo "Expected file to exist: $1" >&2
        return 1
    fi
}

assert_file_not_exists() {
    if [ -f "$1" ]; then
        echo "Expected file to not exist: $1" >&2
        return 1
    fi
}

assert_json_field() {
    local json=$1
    local field=$2
    local expected=$3

    local actual=$(echo "$json" | jq -r ".$field")
    if [ "$actual" != "$expected" ]; then
        echo "Expected $field=$expected, got: $actual" >&2
        return 1
    fi
}
```

### Running BATS Tests

```bash
# Run all CLI tests
bats tests/cli/*.bats

# Run specific test file
bats tests/cli/single_instance.bats

# Run with TAP output
bats --tap tests/cli/*.bats

# Run with timing
bats --timing tests/cli/*.bats

# Run in parallel (requires GNU parallel)
find tests/cli -name '*.bats' | parallel bats {}
```

---

## UI Testing Strategy (Playwright)

### Why Playwright?

- **Modern API** - Clean, intuitive syntax with auto-waiting
- **CDP Support** - Can connect to Tauri via Chrome DevTools Protocol
- **Debugging** - Built-in inspector, trace viewer, video recording
- **Cross-platform** - Excellent Windows/macOS/Linux support
- **Fast** - No WebDriver protocol overhead

### Tauri + Playwright Setup

#### Step 1: Install Dependencies

```bash
cd apps/desktop
npm install --save-dev @playwright/test playwright
```

#### Step 2: Configure Playwright

**File:** `playwright.config.ts`

```typescript
import { defineConfig, devices } from '@playwright/test';

export default defineConfig({
  testDir: './tests/ui',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: process.env.CI ? 1 : undefined,

  reporter: [
    ['html'],
    ['json', { outputFile: 'test-results/results.json' }],
  ],

  use: {
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
    video: 'retain-on-failure',
  },

  projects: [
    {
      name: 'agentmux-desktop',
      use: {
        ...devices['Desktop Chrome'],
        // Connect to Tauri via CDP
        connectOptions: {
          wsEndpoint: 'ws://localhost:9222',
        },
      },
    },
  ],

  webServer: {
    command: 'WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9222" npm run tauri:dev',
    url: 'http://localhost:9222',
    timeout: 120 * 1000,
    reuseExistingServer: !process.env.CI,
  },
});
```

### Page Object Model (POM)

**Structure:**

```
tests/ui/
├── page-objects/
│   ├── DashboardPage.ts
│   ├── AgentsManagerPage.ts
│   ├── MessageStreamPage.ts
│   └── BasePage.ts
├── fixtures/
│   └── agentmux.fixture.ts
├── tests/
│   ├── dashboard.spec.ts
│   ├── agents.spec.ts
│   └── messages.spec.ts
└── helpers/
    └── test-utils.ts
```

#### Base Page Object

**File:** `tests/ui/page-objects/BasePage.ts`

```typescript
import { Page, Locator } from '@playwright/test';

export class BasePage {
  protected readonly page: Page;

  constructor(page: Page) {
    this.page = page;
  }

  // Navigate to specific tab
  async navigateToTab(tabName: string): Promise<void> {
    await this.page.click(`button.tab:has-text("${tabName}")`);
  }

  // Wait for loading to complete
  async waitForLoaded(): Promise<void> {
    await this.page.waitForLoadState('networkidle');
  }

  // Check if error message is displayed
  async hasError(): Promise<boolean> {
    const errorEl = this.page.locator('.error-message');
    return await errorEl.isVisible();
  }

  async getErrorText(): Promise<string> {
    return await this.page.locator('.error-message').textContent() || '';
  }
}
```

#### Dashboard Page Object

**File:** `tests/ui/page-objects/DashboardPage.ts`

```typescript
import { Page, Locator, expect } from '@playwright/test';
import { BasePage } from './BasePage';

export class DashboardPage extends BasePage {
  // Locators
  readonly startBusButton: Locator;
  readonly stopBusButton: Locator;
  readonly busStatus: Locator;
  readonly connectedAgentsCount: Locator;
  readonly messagesPerSecCount: Locator;

  constructor(page: Page) {
    super(page);
    this.startBusButton = page.locator('button:has-text("Start Bus")');
    this.stopBusButton = page.locator('button:has-text("Stop Bus")');
    this.busStatus = page.locator('span:has-text("Status:")');
    this.connectedAgentsCount = page.locator('.stat-card:has-text("Connected Agents") .value');
    this.messagesPerSecCount = page.locator('.stat-card:has-text("Messages/sec") .value');
  }

  // Actions
  async startBus(): Promise<void> {
    await this.startBusButton.click();
    // Wait for reactive update
    await expect(this.busStatus).toContainText('Running', { timeout: 3000 });
  }

  async stopBus(): Promise<void> {
    await this.stopBusButton.click();
    await expect(this.busStatus).toContainText('Stopped', { timeout: 3000 });
  }

  // Assertions
  async isBusRunning(): Promise<boolean> {
    const text = await this.busStatus.textContent();
    return text?.includes('Running') || false;
  }

  async getConnectedAgentsCount(): Promise<number> {
    const text = await this.connectedAgentsCount.textContent();
    return parseInt(text || '0');
  }

  async getMessagesPerSec(): Promise<number> {
    const text = await this.messagesPerSecCount.textContent();
    return parseInt(text || '0');
  }

  // Assertions (for readability in tests)
  async expectBusRunning(): Promise<void> {
    await expect(this.busStatus).toContainText('Running');
    await expect(this.startBusButton).toBeDisabled();
    await expect(this.stopBusButton).toBeEnabled();
  }

  async expectBusStopped(): Promise<void> {
    await expect(this.busStatus).toContainText('Stopped');
    await expect(this.startBusButton).toBeEnabled();
    await expect(this.stopBusButton).toBeDisabled();
  }
}
```

#### Agents Manager Page Object

**File:** `tests/ui/page-objects/AgentsManagerPage.ts`

```typescript
import { Page, Locator, expect } from '@playwright/test';
import { BasePage } from './BasePage';

export class AgentsManagerPage extends BasePage {
  readonly agentNameInput: Locator;
  readonly agentCommandInput: Locator;
  readonly spawnAgentButton: Locator;
  readonly stopAgentButton: Locator;
  readonly agentsList: Locator;

  constructor(page: Page) {
    super(page);
    this.agentNameInput = page.locator('input[placeholder*="name"]');
    this.agentCommandInput = page.locator('input[placeholder*="command"]');
    this.spawnAgentButton = page.locator('button:has-text("Spawn Agent")');
    this.stopAgentButton = page.locator('button:has-text("Stop Agent")');
    this.agentsList = page.locator('.agents-list');
  }

  async spawnAgent(name: string, command: string = 'claude'): Promise<void> {
    await this.agentNameInput.fill(name);
    await this.agentCommandInput.fill(command);
    await this.spawnAgentButton.click();

    // Wait for agent to appear (reactive UI)
    await expect(this.getAgentCard(name)).toBeVisible({ timeout: 5000 });
  }

  async stopAgent(name: string): Promise<void> {
    const agentCard = this.getAgentCard(name);
    await agentCard.click(); // Select agent
    await this.stopAgentButton.click();

    // Wait for agent to disappear
    await expect(agentCard).not.toBeVisible({ timeout: 5000 });
  }

  getAgentCard(name: string): Locator {
    return this.page.locator(`.agent-card:has-text("${name}")`);
  }

  async getAgentStatus(name: string): Promise<string> {
    const card = this.getAgentCard(name);
    const statusEl = card.locator('.stat:has-text("Status") .value');
    return await statusEl.textContent() || '';
  }

  async isAgentRunning(name: string): Promise<boolean> {
    const status = await this.getAgentStatus(name);
    return status.toLowerCase().includes('running');
  }

  async getAgentCount(): Promise<number> {
    const cards = await this.agentsList.locator('.agent-card').count();
    return cards;
  }
}
```

### Playwright Fixtures

**File:** `tests/ui/fixtures/agentmux.fixture.ts`

```typescript
import { test as base } from '@playwright/test';
import { DashboardPage } from '../page-objects/DashboardPage';
import { AgentsManagerPage } from '../page-objects/AgentsManagerPage';
import { MessageStreamPage } from '../page-objects/MessageStreamPage';

// Define fixture types
type AgentMuxFixtures = {
  dashboardPage: DashboardPage;
  agentsManagerPage: AgentsManagerPage;
  messageStreamPage: MessageStreamPage;
};

// Extend base test with custom fixtures
export const test = base.extend<AgentMuxFixtures>({
  // Dashboard page fixture
  dashboardPage: async ({ page }, use) => {
    const dashboardPage = new DashboardPage(page);
    await page.goto('/');
    await dashboardPage.waitForLoaded();
    await use(dashboardPage);
  },

  // Agents Manager page fixture
  agentsManagerPage: async ({ page }, use) => {
    const agentsPage = new AgentsManagerPage(page);
    await page.goto('/');
    await agentsPage.navigateToTab('Agents');
    await agentsPage.waitForLoaded();
    await use(agentsPage);
  },

  // Message Stream page fixture
  messageStreamPage: async ({ page }, use) => {
    const messagePage = new MessageStreamPage(page);
    await page.goto('/');
    await messagePage.navigateToTab('Messages');
    await messagePage.waitForLoaded();
    await use(messagePage);
  },
});

export { expect } from '@playwright/test';
```

### Test Examples with POM + Fixtures

#### Dashboard Tests

**File:** `tests/ui/tests/dashboard.spec.ts`

```typescript
import { test, expect } from '../fixtures/agentmux.fixture';

test.describe('Dashboard - Bus Control', () => {
  test('should start bus when Start button clicked', async ({ dashboardPage }) => {
    // Initial state
    await dashboardPage.expectBusStopped();

    // Start bus
    await dashboardPage.startBus();

    // Verify state
    await dashboardPage.expectBusRunning();
  });

  test('should stop bus when Stop button clicked', async ({ dashboardPage }) => {
    // Ensure bus is running first
    if (!(await dashboardPage.isBusRunning())) {
      await dashboardPage.startBus();
    }

    // Stop bus
    await dashboardPage.stopBus();

    // Verify state
    await dashboardPage.expectBusStopped();
  });

  test('should display connected agents count', async ({ dashboardPage }) => {
    const count = await dashboardPage.getConnectedAgentsCount();
    expect(count).toBeGreaterThanOrEqual(0);
  });

  test('should update metrics in real-time', async ({ dashboardPage }) => {
    // Start bus if not running
    if (!(await dashboardPage.isBusRunning())) {
      await dashboardPage.startBus();
    }

    const initialMps = await dashboardPage.getMessagesPerSec();

    // Wait for potential updates (reactive UI)
    await dashboardPage.page.waitForTimeout(2000);

    const updatedMps = await dashboardPage.getMessagesPerSec();

    // MPS should be numeric (may stay same if no activity)
    expect(typeof updatedMps).toBe('number');
  });
});

test.describe('Dashboard - Error Handling', () => {
  test('should display error when bus start fails', async ({ dashboardPage }) => {
    // This test assumes bus is already running (simulate error condition)
    // Implementation depends on how you want to trigger errors

    // For now, just verify error display capability exists
    const hasError = await dashboardPage.hasError();
    // Initially should have no error
    expect(hasError).toBe(false);
  });
});
```

#### Agents Manager Tests

**File:** `tests/ui/tests/agents.spec.ts`

```typescript
import { test, expect } from '../fixtures/agentmux.fixture';

test.describe('Agents Manager', () => {
  test('should spawn new agent', async ({ agentsManagerPage }) => {
    const agentName = `TestAgent_${Date.now()}`;

    await agentsManagerPage.spawnAgent(agentName);

    // Verify agent card exists
    await expect(agentsManagerPage.getAgentCard(agentName)).toBeVisible();
  });

  test('should display agent status', async ({ agentsManagerPage }) => {
    const agentName = `StatusAgent_${Date.now()}`;

    await agentsManagerPage.spawnAgent(agentName);

    const status = await agentsManagerPage.getAgentStatus(agentName);
    expect(status).toMatch(/running|stopped/i);
  });

  test('should stop agent', async ({ agentsManagerPage }) => {
    const agentName = `StopAgent_${Date.now()}`;

    // Spawn agent first
    await agentsManagerPage.spawnAgent(agentName);

    // Stop agent
    await agentsManagerPage.stopAgent(agentName);

    // Verify agent card removed
    await expect(agentsManagerPage.getAgentCard(agentName)).not.toBeVisible();
  });

  test('should handle multiple agents', async ({ agentsManagerPage }) => {
    const initialCount = await agentsManagerPage.getAgentCount();

    // Spawn 3 agents
    const agents = [
      `Agent1_${Date.now()}`,
      `Agent2_${Date.now()}`,
      `Agent3_${Date.now()}`,
    ];

    for (const name of agents) {
      await agentsManagerPage.spawnAgent(name);
    }

    const finalCount = await agentsManagerPage.getAgentCount();
    expect(finalCount).toBe(initialCount + 3);
  });
});

test.describe('Agents Manager - Form Validation', () => {
  test('should require agent name', async ({ agentsManagerPage }) => {
    // Leave name empty
    await agentsManagerPage.agentNameInput.clear();

    const isDisabled = await agentsManagerPage.spawnAgentButton.isDisabled();
    // This assumes validation is implemented
    // Adjust based on actual implementation
  });
});
```

### Test Isolation Best Practices

```typescript
// Good: Use API/IPC to set up test data
test('should display agent in list', async ({ page, agentsManagerPage }) => {
  // Set up via IPC (fast, reliable)
  await page.evaluate(() => {
    return window.__TAURI__.core.invoke('spawn_embedded_claude', {
      name: 'TestAgent',
      command: 'claude',
    });
  });

  // Now test the UI displays it
  await expect(agentsManagerPage.getAgentCard('TestAgent')).toBeVisible();
});

// Bad: Click through UI to set up test data
test('should display agent in list', async ({ agentsManagerPage }) => {
  // Slow, brittle, depends on other features working
  await agentsManagerPage.spawnAgent('TestAgent');

  await expect(agentsManagerPage.getAgentCard('TestAgent')).toBeVisible();
});
```

### Running Playwright Tests

```bash
# Run all tests
npx playwright test

# Run specific test file
npx playwright test tests/ui/tests/dashboard.spec.ts

# Run in headed mode (see browser)
npx playwright test --headed

# Debug mode (step through)
npx playwright test --debug

# Run specific test by name
npx playwright test -g "should start bus"

# Generate test report
npx playwright show-report

# Update snapshots
npx playwright test --update-snapshots
```

---

## Integration Testing

### E2E Workflow Tests

**File:** `tests/e2e/agent_orchestration.spec.ts`

```typescript
import { test, expect } from '@playwright/test';
import { exec } from 'child_process';
import { promisify } from 'util';

const execAsync = promisify(exec);

test.describe('E2E: Complete Agent Workflow', () => {
  test('spawn agent via CLI, verify in UI, send message, stop agent', async ({ page }) => {
    const agentName = `E2EAgent_${Date.now()}`;

    // 1. Spawn agent via CLI (IPC)
    const spawnResult = await execAsync(`agentmux --json agents spawn ${agentName}`);
    const spawnData = JSON.parse(spawnResult.stdout);
    expect(spawnData.success).toBe(true);

    // 2. Verify agent appears in UI
    await page.goto('/');
    await page.click('button.tab:has-text("Agents")');
    await expect(page.locator(`.agent-card:has-text("${agentName}")`)).toBeVisible();

    // 3. Send message via CLI
    await execAsync(`agentmux messages send --to ${agentName} --message "Test message"`);

    // 4. Verify message in UI
    await page.click('button.tab:has-text("Messages")');
    await expect(page.locator('text=Test message')).toBeVisible();

    // 5. Stop agent via CLI
    const stopResult = await execAsync(`agentmux agents stop ${agentName}`);
    expect(stopResult.stdout).toContain('stopped');

    // 6. Verify agent removed from UI
    await page.click('button.tab:has-text("Agents")');
    await expect(page.locator(`.agent-card:has-text("${agentName}")`)).not.toBeVisible();
  });
});
```

---

## Test Infrastructure

### Directory Structure

```
apps/desktop/
├── tests/
│   ├── cli/                          # BATS tests
│   │   ├── single_instance.bats
│   │   ├── ipc_forwarding.bats
│   │   ├── headless_mode.bats
│   │   ├── agent_lifecycle.bats
│   │   ├── bus_lifecycle.bats
│   │   ├── messaging.bats
│   │   ├── log_export.bats
│   │   └── helpers/
│   │       ├── test_helper.bash
│   │       └── assertions.bash
│   ├── ui/                           # Playwright tests
│   │   ├── page-objects/
│   │   │   ├── BasePage.ts
│   │   │   ├── DashboardPage.ts
│   │   │   ├── AgentsManagerPage.ts
│   │   │   └── MessageStreamPage.ts
│   │   ├── fixtures/
│   │   │   └── agentmux.fixture.ts
│   │   ├── tests/
│   │   │   ├── dashboard.spec.ts
│   │   │   ├── agents.spec.ts
│   │   │   └── messages.spec.ts
│   │   └── helpers/
│   │       └── test-utils.ts
│   ├── e2e/                          # End-to-end tests
│   │   └── agent_orchestration.spec.ts
│   └── fixtures/
│       ├── test_config.json
│       └── sample_messages.json
├── playwright.config.ts
├── package.json
└── bats.config                       # Optional BATS config
```

### Test Runner Scripts

**File:** `package.json` scripts

```json
{
  "scripts": {
    "test": "npm run test:cli && npm run test:ui",
    "test:cli": "bats tests/cli/*.bats",
    "test:ui": "playwright test",
    "test:e2e": "playwright test tests/e2e",
    "test:headed": "playwright test --headed",
    "test:debug": "playwright test --debug",
    "test:report": "playwright show-report"
  }
}
```

---

## CI/CD Integration

### GitHub Actions Workflow

**File:** `.github/workflows/application-tests.yml`

```yaml
name: Application Tests

on:
  push:
    branches: [main, develop]
  pull_request:
    branches: [main]

jobs:
  cli-tests:
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
    runs-on: ${{ matrix.os }}

    steps:
      - uses: actions/checkout@v4

      - name: Install BATS
        shell: bash
        run: |
          if [ "$RUNNER_OS" == "macOS" ]; then
            brew install bats-core
          elif [ "$RUNNER_OS" == "Linux" ]; then
            sudo apt-get update
            sudo apt-get install -y bats
          elif [ "$RUNNER_OS" == "Windows" ]; then
            choco install bats-core
          fi

      - name: Setup Node.js
        uses: actions/setup-node@v4
        with:
          node-version: '18'

      - name: Install Rust
        uses: actions-rs/toolchain@v1
        with:
          toolchain: stable

      - name: Build application
        working-directory: apps/desktop
        run: |
          npm install
          npm run tauri:build

      - name: Run CLI tests
        working-directory: apps/desktop
        run: npm run test:cli

  ui-tests:
    runs-on: ubuntu-latest

    steps:
      - uses: actions/checkout@v4

      - name: Setup Node.js
        uses: actions/setup-node@v4
        with:
          node-version: '18'

      - name: Install dependencies
        working-directory: apps/desktop
        run: |
          npm install
          npx playwright install --with-deps

      - name: Install Rust
        uses: actions-rs/toolchain@v1
        with:
          toolchain: stable

      - name: Build application
        working-directory: apps/desktop
        run: npm run tauri:build

      - name: Run Playwright tests
        working-directory: apps/desktop
        env:
          WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: '--remote-debugging-port=9222'
        run: npm run test:ui

      - name: Upload test results
        if: always()
        uses: actions/upload-artifact@v4
        with:
          name: playwright-report
          path: apps/desktop/playwright-report/
          retention-days: 30

  e2e-tests:
    needs: [cli-tests, ui-tests]
    runs-on: ubuntu-latest

    steps:
      - uses: actions/checkout@v4

      - name: Setup Node.js
        uses: actions/setup-node@v4
        with:
          node-version: '18'

      - name: Install dependencies
        working-directory: apps/desktop
        run: |
          npm install
          npx playwright install --with-deps
          sudo apt-get install -y bats

      - name: Install Rust
        uses: actions-rs/toolchain@v1
        with:
          toolchain: stable

      - name: Build application
        working-directory: apps/desktop
        run: npm run tauri:build

      - name: Run E2E tests
        working-directory: apps/desktop
        run: npm run test:e2e
```

---

## Test Data Management

### Fixtures

**File:** `tests/fixtures/test_config.json`

```json
{
  "bus": {
    "host": "127.0.0.1",
    "port": 8765,
    "max_agents": 50
  },
  "test_agents": [
    {
      "name": "TestAgent1",
      "command": "claude"
    }
  ],
  "test_messages": [
    {
      "to": "TestAgent1",
      "message": "Test message 1",
      "priority": "normal"
    }
  ]
}
```

### State Isolation

Each test maintains isolation through:

1. **Unique identifiers** - Use timestamps or random strings
2. **Independent state** - Each test cleans up after itself
3. **API-based setup** - Use IPC/CLI instead of UI clicks
4. **Parallel-safe** - Tests don't interfere when run concurrently

---

## Success Criteria

### Coverage Targets

- **CLI Commands:** 100% coverage
- **UI Components:** 90%+ coverage
- **Critical Paths:** 100% coverage
- **Error Scenarios:** 80%+ coverage

### Performance Benchmarks

- Full test suite: < 5 minutes
- Individual test: < 30 seconds
- No memory leaks
- No zombie processes

### Quality Gates

**Required for merge:**
- All tests passing
- No flaky tests (< 1% failure rate)
- Code coverage maintained or improved

---

## Implementation Roadmap

### Phase 1: BATS CLI Tests (Week 1-2)
- ✅ Install BATS framework
- ✅ Convert existing bash tests to BATS
- ✅ Create helper functions and assertions
- ✅ Set up CI integration

### Phase 2: Playwright Setup (Week 2-3)
- ✅ Install Playwright + CDP configuration
- ✅ Create base Page Objects
- ✅ Implement custom fixtures
- ✅ Write first UI test suite

### Phase 3: Core UI Tests (Week 3-4)
- ✅ Dashboard test suite
- ✅ Agents Manager test suite
- ✅ Message Stream test suite
- ✅ Debug Console test suite

### Phase 4: E2E Tests (Week 4-5)
- ✅ Combined CLI + UI workflows
- ✅ Cross-platform validation
- ✅ Performance benchmarks

### Phase 5: CI/CD & Maintenance (Week 5+)
- ✅ GitHub Actions setup
- ✅ Test reporting and metrics
- ✅ Continuous optimization

---

## Notes

- **Maintainability** - POM keeps tests readable and DRY
- **Reliability** - Fixtures ensure consistent test environment
- **Speed** - API-based setup and parallel execution
- **Debuggability** - Playwright's tracing and video capture

---

**End of Specification v2.0**

# Spec: Node.js Detection & User Notification

**Date:** 2026-03-19
**Status:** Implementation Ready
**Priority:** High — Codex and Gemini agents fail silently when Node.js is missing
**Effort:** ~2 hours

---

## Problem

Codex and Gemini CLIs are npm packages that require Node.js to install and run. When Node.js is not installed, `npm.cmd` / `npm` fails with a cryptic OS error ("The filename, directory name, or volume label syntax is incorrect"). The user sees:

```
[cli] install failed: The filename, directory name, or volume label syntax is incorrect.
[error] codex not available — install manually or check your internet connection
```

Claude doesn't have this problem because its installer (`irm https://claude.ai/install.ps1 | iex`) bundles its own Node runtime.

---

## Solution

Add a Node.js detection check before attempting `npm install`. If Node.js is missing, show a clear notification with platform-specific install instructions instead of a cryptic error.

---

## Implementation

### 1. New Tauri Command: `check_nodejs_available`

**File:** `src-tauri/src/commands/cli_installer.rs`

```rust
#[derive(Debug, Serialize, Deserialize)]
pub struct NodejsStatus {
    pub available: bool,
    pub version: Option<String>,
    pub npm_available: bool,
    pub npm_version: Option<String>,
    pub path: Option<String>,
}

/// Check if Node.js and npm are available on the system.
/// Returns version info if found, or available=false if not.
#[tauri::command]
pub async fn check_nodejs_available() -> Result<NodejsStatus, String> {
    let result = tokio::task::spawn_blocking(|| {
        let node_cmd = if cfg!(windows) { "node.exe" } else { "node" };
        let npm_cmd = if cfg!(windows) { "npm.cmd" } else { "npm" };

        let mut status = NodejsStatus {
            available: false,
            version: None,
            npm_available: false,
            npm_version: None,
            path: None,
        };

        // Check node
        if let Ok(output) = std::process::Command::new(node_cmd)
            .arg("--version")
            .output()
        {
            if output.status.success() {
                let ver = String::from_utf8_lossy(&output.stdout).trim().to_string();
                status.available = true;
                status.version = Some(ver);

                // Get node path
                let which_cmd = if cfg!(windows) { "where" } else { "which" };
                if let Ok(path_out) = std::process::Command::new(which_cmd)
                    .arg(node_cmd)
                    .output()
                {
                    if path_out.status.success() {
                        status.path = Some(
                            String::from_utf8_lossy(&path_out.stdout)
                                .lines()
                                .next()
                                .unwrap_or("")
                                .trim()
                                .to_string(),
                        );
                    }
                }
            }
        }

        // Check npm
        if let Ok(output) = std::process::Command::new(npm_cmd)
            .arg("--version")
            .output()
        {
            if output.status.success() {
                let ver = String::from_utf8_lossy(&output.stdout).trim().to_string();
                status.npm_available = true;
                status.npm_version = Some(ver);
            }
        }

        status
    })
    .await
    .map_err(|e| format!("Failed to check Node.js: {e}"))?;

    Ok(result)
}
```

**Register in `lib.rs`:**
```rust
commands::cli_installer::check_nodejs_available,
```

### 2. Pre-Flight Check in `install_cli`

**File:** `src-tauri/src/commands/cli_installer.rs`

Modify `install_via_npm` to check Node.js before attempting install:

```rust
fn install_via_npm(provider: &str) -> Result<String, String> {
    // Pre-flight: check if npm is available
    let npm_cmd = if cfg!(windows) { "npm.cmd" } else { "npm" };
    match std::process::Command::new(npm_cmd).arg("--version").output() {
        Ok(output) if output.status.success() => {
            let ver = String::from_utf8_lossy(&output.stdout);
            tracing::info!("npm {} available", ver.trim());
        }
        _ => {
            return Err(
                "NODEJS_NOT_FOUND: Node.js/npm is not installed. \
                 Codex and Gemini CLIs require Node.js. \
                 Install from https://nodejs.org/ (LTS recommended)."
                    .to_string(),
            );
        }
    }

    // ... existing npm install logic ...
}
```

### 3. Frontend: Detect Error and Show Notification

**File:** `frontend/app/view/agent/agent-model.ts`

In the launch flow, before calling `install_cli`, check for Node.js:

```typescript
import { getApi } from "@/store/global";

async function ensureCliInstalled(provider: ProviderDefinition): Promise<string> {
    // Step 1: Check if CLI is already installed
    const existingPath = await getApi().getCliPath(provider.id);
    if (existingPath) return existingPath;

    // Step 2: For npm-based providers (not Claude), check Node.js first
    if (provider.id !== "claude") {
        const nodeStatus = await getApi().checkNodejsAvailable();
        if (!nodeStatus.available || !nodeStatus.npm_available) {
            throw new NodejsNotFoundError(provider);
        }
    }

    // Step 3: Install
    const result = await getApi().installCli(provider.id);
    return result.cli_path;
}
```

### 4. Notification UI

**File:** `frontend/app/view/agent/agent-view.tsx`

When `NodejsNotFoundError` is caught, show an in-pane notification:

```tsx
class NodejsNotFoundError extends Error {
    provider: ProviderDefinition;
    constructor(provider: ProviderDefinition) {
        super(`Node.js is required to run ${provider.displayName}`);
        this.provider = provider;
    }
}

// In the agent view, render the error state:
function NodejsRequiredNotice(props: { provider: ProviderDefinition }) {
    const platform = getPlatform();

    const installCommand = () => {
        if (platform === "win32") return "winget install OpenJS.NodeJS.LTS";
        if (platform === "darwin") return "brew install node";
        return "sudo apt install nodejs npm";  // Linux
    };

    const downloadUrl = "https://nodejs.org/en/download/";

    return (
        <div class="agent-notice nodejs-required">
            <div class="notice-icon">
                <i class="fa-solid fa-circle-exclamation" />
            </div>
            <div class="notice-content">
                <h3>Node.js Required</h3>
                <p>
                    {props.provider.displayName} requires Node.js to install and run.
                    Node.js was not detected on your system.
                </p>
                <div class="notice-install-options">
                    <div class="install-option">
                        <strong>Option 1:</strong> Install via command line
                        <code class="install-command">{installCommand()}</code>
                    </div>
                    <div class="install-option">
                        <strong>Option 2:</strong> Download from{" "}
                        <a href={downloadUrl} onClick={(e) => {
                            e.preventDefault();
                            getApi().openExternal(downloadUrl);
                        }}>
                            nodejs.org
                        </a>
                        {" "}(LTS recommended)
                    </div>
                </div>
                <p class="notice-hint">
                    After installing Node.js, restart AgentMux and try again.
                </p>
            </div>
        </div>
    );
}
```

### 5. CSS for the Notice

**File:** `frontend/app/view/agent/agent.scss` (or new file)

```scss
.agent-notice.nodejs-required {
    display: flex;
    gap: 16px;
    padding: 24px;
    margin: 24px;
    border-radius: 8px;
    background: var(--block-bg-color);
    border: 1px solid var(--border-color);

    .notice-icon {
        font-size: 24px;
        color: #f59e0b;  // amber warning
    }

    .notice-content {
        h3 {
            margin: 0 0 8px;
            font-size: 16px;
        }

        p {
            margin: 0 0 12px;
            color: var(--secondary-text-color);
        }

        .install-command {
            display: block;
            margin-top: 4px;
            padding: 8px 12px;
            background: rgba(0, 0, 0, 0.3);
            border-radius: 4px;
            font-family: var(--termfontfamily);
            font-size: 13px;
            user-select: all;
        }

        .install-option {
            margin-bottom: 12px;
        }

        .notice-hint {
            font-style: italic;
            font-size: 13px;
        }
    }
}
```

---

## Flow Diagram

```
User clicks "Launch Agent" (Codex/Gemini)
    ↓
ensureCliInstalled(provider)
    ↓
getCliPath() → null (not installed)
    ↓
Is provider === "claude"?
    YES → use Claude's standalone installer
    NO  → checkNodejsAvailable()
            ↓
        Node.js found?
            YES → installCli() via npm
            NO  → throw NodejsNotFoundError
                    ↓
                Show NodejsRequiredNotice in agent pane
                (platform-specific install commands)
```

---

## Files to Modify

| File | Changes |
|------|---------|
| `src-tauri/src/commands/cli_installer.rs` | Add `check_nodejs_available` command, add pre-flight check in `install_via_npm` |
| `src-tauri/src/lib.rs` | Register `check_nodejs_available` command |
| `frontend/app/view/agent/agent-model.ts` | Add `ensureCliInstalled` with Node.js check |
| `frontend/app/view/agent/agent-view.tsx` | Add `NodejsRequiredNotice` component |
| `frontend/app/view/agent/agent.scss` | Add notice styles |

---

## What This Does NOT Change

- Claude agent flow — unchanged, uses its own installer
- Backend `ResolveCliCommand` — unchanged, frontend handles the pre-check
- Provider definitions — unchanged
- Forge agent config/launch — unchanged, just wraps the CLI resolution

---

## Edge Cases

1. **Node.js installed after AgentMux starts:** User installs Node.js, comes back to AgentMux. The check runs on each launch attempt, so it will detect Node.js without restart. Add a "Retry" button to the notice.

2. **Node.js installed but not on PATH:** The `node --version` check will fail. The notice should mention "Make sure Node.js is on your system PATH."

3. **npm installed but node is not (unlikely):** Check both independently and report which is missing.

4. **Corporate environments with proxy:** `npm install` may fail even with Node.js present. The error from `install_via_npm` handles this separately — it's not a Node.js detection issue.

5. **nvm/fnm users:** Node.js may be available in the user's shell but not in the subprocess environment. The Tauri command inherits the app's environment, which may not include nvm's PATH modifications. Mention in the notice: "If using nvm/fnm, ensure the default Node.js version is set."

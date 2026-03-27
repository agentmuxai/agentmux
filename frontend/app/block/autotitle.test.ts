// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { assert, describe, test } from "vitest";
import {
    detectAgentFromPath,
    detectAgentFromWorkspacesPath,
    generateAutoTitle,
    getEffectiveTitle,
    shouldAutoGenerateTitle,
} from "./autotitle";

describe("detectAgentFromWorkspacesPath", () => {
    test("detects agent from Unix path", () => {
        const path = "/home/user/agent-workspaces/agent2/wavemux";
        const result = detectAgentFromWorkspacesPath(path);
        assert.equal(result, "Agent2");
    });

    test("detects agent from Windows path", () => {
        const path = "C:\\Code\\agent-workspaces\\agent3\\project";
        const result = detectAgentFromWorkspacesPath(path);
        assert.equal(result, "Agent3");
    });

    test("detects agentx (case-insensitive)", () => {
        const path = "/code/agent-workspaces/agentx/task";
        const result = detectAgentFromWorkspacesPath(path);
        assert.equal(result, "AgentX");
    });

    test("returns null for non-workspace paths", () => {
        const path = "/home/user/projects/myapp";
        const result = detectAgentFromWorkspacesPath(path);
        assert.equal(result, null);
    });

    test("returns null for hostname-based paths (only checks workspaces)", () => {
        // This path would be detected by detectAgentFromPath but NOT by detectAgentFromWorkspacesPath
        const path = "C:\\Systems\\wavemux";
        const result = detectAgentFromWorkspacesPath(path);
        assert.equal(result, null);
    });
});

describe("detectAgentFromPath", () => {
    test("detects agent from Unix path", () => {
        const path = "/home/user/agent-workspaces/agent2/wavemux";
        const result = detectAgentFromPath(path);
        assert.equal(result, "Agent2");
    });

    test("detects agent from Windows path", () => {
        const path = "C:\\Code\\agent-workspaces\\agent3\\project";
        const result = detectAgentFromPath(path);
        assert.equal(result, "Agent3");
    });

    test("detects agentx (case-insensitive)", () => {
        const path = "/code/agent-workspaces/agentx/task";
        const result = detectAgentFromPath(path);
        assert.equal(result, "AgentX");
    });

    test("returns null for non-agent paths", () => {
        const path = "/home/user/projects/myapp";
        const result = detectAgentFromPath(path);
        assert.equal(result, null);
    });

    test("returns null for empty/null paths", () => {
        assert.equal(detectAgentFromPath(undefined), null);
        assert.equal(detectAgentFromPath(""), null);
        assert.equal(detectAgentFromPath(null as any), null);
    });

    test("handles mixed case agent-workspaces", () => {
        const path = "D:\\Code\\Agent-Workspaces\\AGENT5\\src";
        const result = detectAgentFromPath(path);
        assert.equal(result, "Agent5");
    });
});

describe("generateAutoTitle", () => {
    test("generates terminal title from cwd", () => {
        const block: Block = {
            otype: "block",
            oid: "test-123",
            version: 1,
            meta: {
                view: "term",
                "cmd:cwd": "/home/user/projects/myapp",
            },
        };
        const title = generateAutoTitle(block);
        assert.equal(title, "myapp");
    });

    test("generates terminal title with agent identity from agent workspace", () => {
        const block: Block = {
            otype: "block",
            oid: "test-123",
            version: 1,
            meta: {
                view: "term",
                "cmd:cwd": "C:\\Code\\agent-workspaces\\agent2\\wavemux",
            },
        };
        const title = generateAutoTitle(block);
        assert.equal(title, "Agent2");
    });

    test("terminal does NOT use hostname-based detection", () => {
        // Hostname-based detection has been removed entirely
        // Agent identity should ONLY come from explicit env vars
        const block: Block = {
            otype: "block",
            oid: "test-123",
            version: 1,
            meta: {
                view: "term",
                "cmd:cwd": "C:\\Systems\\wavemux",
            },
        };
        const title = generateAutoTitle(block);
        // Should return directory basename, NOT "AgentA"
        assert.equal(title, "wavemux");
    });

    test("SSH connection also does NOT use hostname-based detection", () => {
        // Even SSH connections don't infer agent from hostname anymore
        // Agent identity must be set explicitly via env vars
        const block: Block = {
            otype: "block",
            oid: "test-123",
            version: 1,
            meta: {
                view: "term",
                "cmd:cwd": "/home/user/agentmux",
                connection: "ssh:devserver",
            },
        };
        const title = generateAutoTitle(block);
        // Should return directory basename, NOT "AgentA" - no hostname inference
        assert.equal(title, "agentmux");
    });

    test("block-level cmd:env IS used for agent identity (set via OSC 16162)", () => {
        const block: Block = {
            otype: "block",
            oid: "test-123",
            version: 1,
            meta: {
                view: "term",
                "cmd:cwd": "C:\\Code\\agent-workspaces\\agent2\\wavemux",
                "cmd:env": { AGENTMUX_AGENT_ID: "BlockAgent" },
            },
        };
        const title = generateAutoTitle(block);
        // Block env takes priority - set via OSC 16162 from shell integration
        assert.equal(title, "BlockAgent");
    });

    test("settings-level env var sets agent identity", () => {
        const block: Block = {
            otype: "block",
            oid: "test-123",
            version: 1,
            meta: {
                view: "term",
                "cmd:cwd": "C:\\Code\\agent-workspaces\\agent2\\wavemux",
            },
        };
        const settingsEnv = { AGENTMUX_AGENT_ID: "SettingsAgent" };
        const title = generateAutoTitle(block, settingsEnv);
        // Settings env takes priority over path detection
        assert.equal(title, "SettingsAgent");
    });

    // Note: shell:lastcmd tests removed - that data is in RTInfo, not metadata
    // TODO: Add RTInfo support and restore these tests

    test("generates preview title from filename", () => {
        const block: Block = {
            otype: "block",
            oid: "test-123",
            version: 1,
            meta: {
                view: "preview",
                file: "/docs/README.md",
            },
        };
        const title = generateAutoTitle(block);
        assert.equal(title, "README.md");
    });

    test("generates preview title from URL", () => {
        const block: Block = {
            otype: "block",
            oid: "test-123",
            version: 1,
            meta: {
                view: "preview",
                url: "https://example.com/page.html",
            },
        };
        const title = generateAutoTitle(block);
        assert.equal(title, "example.com");
    });

    test("generates editor title with parent directory", () => {
        const block: Block = {
            otype: "block",
            oid: "test-123",
            version: 1,
            meta: {
                view: "codeeditor",
                file: "/home/user/projects/src/index.ts",
            },
        };
        const title = generateAutoTitle(block);
        assert.equal(title, "Editor");
    });

    test("generates editor title for short path", () => {
        const block: Block = {
            otype: "block",
            oid: "test-123",
            version: 1,
            meta: {
                view: "codeeditor",
                file: "index.ts",
            },
        };
        const title = generateAutoTitle(block);
        assert.equal(title, "Editor");
    });

    test("generates chat title with channel", () => {
        const block: Block = {
            otype: "block",
            oid: "test-123",
            version: 1,
            meta: {
                view: "chat",
                "chat:channel": "general",
            } as MetaType,
        };
        const title = generateAutoTitle(block);
        assert.equal(title, "Chat: general");
    });

    test("generates default title for help view", () => {
        const block: Block = {
            otype: "block",
            oid: "test-123",
            version: 1,
            meta: {
                view: "help",
            },
        };
        const title = generateAutoTitle(block);
        assert.equal(title, "Help");
    });

    test("generates default title for unknown view", () => {
        const block: Block = {
            otype: "block",
            oid: "test-abcd1234",
            version: 1,
            meta: {
                view: "unknownview",
            },
        };
        const title = generateAutoTitle(block);
        assert.equal(title, "Unknownview (test-abc)");
    });

    test("handles null or empty block gracefully", () => {
        const block: Block = {
            otype: "block",
            oid: "test-123",
            version: 1,
            meta: {},
        };
        const title = generateAutoTitle(block);
        assert.equal(title, "Block (test-123)");
    });
});

describe("shouldAutoGenerateTitle", () => {
    test("returns false when block has custom title", () => {
        const block: Block = {
            otype: "block",
            oid: "test-123",
            version: 1,
            meta: {
                view: "term",
                "pane-title": "My Custom Title",
            } as MetaType,
        };
        const result = shouldAutoGenerateTitle(block);
        assert.equal(result, false);
    });

    test("returns true when block has no custom title", () => {
        const block: Block = {
            otype: "block",
            oid: "test-123",
            version: 1,
            meta: {
                view: "term",
            },
        };
        const result = shouldAutoGenerateTitle(block);
        assert.equal(result, true);
    });

    test("respects explicit auto-generate flag (true)", () => {
        const block: Block = {
            otype: "block",
            oid: "test-123",
            version: 1,
            meta: {
                view: "term",
                "pane-title": "Custom",
                "pane-title:auto": true,
            } as MetaType,
        };
        const result = shouldAutoGenerateTitle(block);
        assert.equal(result, true);
    });

    test("respects explicit auto-generate flag (false)", () => {
        const block: Block = {
            otype: "block",
            oid: "test-123",
            version: 1,
            meta: {
                view: "term",
                "pane-title:auto": false,
            } as MetaType,
        };
        const result = shouldAutoGenerateTitle(block);
        assert.equal(result, false);
    });

    test("handles null block safely", () => {
        const block: any = null;
        const result = shouldAutoGenerateTitle(block);
        assert.equal(result, false);
    });
});

describe("getEffectiveTitle", () => {
    test("returns custom title when set", () => {
        const block: Block = {
            otype: "block",
            oid: "test-123",
            version: 1,
            meta: {
                view: "term",
                "pane-title": "My Terminal",
                "cmd:cwd": "/home/user",
            } as MetaType,
        };
        const title = getEffectiveTitle(block, true);
        assert.equal(title, "My Terminal");
    });

    test("returns auto-generated title when no custom title", () => {
        const block: Block = {
            otype: "block",
            oid: "test-123",
            version: 1,
            meta: {
                view: "preview",
                file: "README.md",
            },
        };
        const title = getEffectiveTitle(block, true);
        assert.equal(title, "README.md");
    });

    test("returns empty string when auto-generate disabled", () => {
        const block: Block = {
            otype: "block",
            oid: "test-123",
            version: 1,
            meta: {
                view: "term",
                "cmd:cwd": "/home/user",
            },
        };
        const title = getEffectiveTitle(block, false);
        assert.equal(title, "");
    });

    test("prefers custom title even when auto-generate enabled", () => {
        const block: Block = {
            otype: "block",
            oid: "test-123",
            version: 1,
            meta: {
                view: "term",
                "pane-title": "Custom",
                "cmd:cwd": "/home/user",
            } as MetaType,
        };
        const title = getEffectiveTitle(block, true);
        assert.equal(title, "Custom");
    });

    test("handles null block safely", () => {
        const block: any = null;
        const title = getEffectiveTitle(block, true);
        assert.equal(title, "");
    });
});

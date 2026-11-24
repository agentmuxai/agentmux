// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A prompt that speaks for the person (their answer, approval or consent) is marked `data-operator-only`, which the
 * UI tool's click and key paths refuse. This test finds every source file that calls a command only the person's
 * window may send (`OPERATOR_COMMANDS`) and fails for one that is on neither list below, so a new prompt cannot be
 * added unmarked: the author has to say what it is.
 */

import { readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { OPERATOR_COMMANDS } from "./operator-proof";

const FRONTEND = join(dirname(fileURLToPath(import.meta.url)), "..", "..");

/** Files that call a person-only command to carry out a prompt's answer, with the files that draw that prompt. Each must carry the marker. */
const PROMPTS: Record<string, string[]> = {
    "app/view/agent/hooks/useAgentDecisions.ts": ["app/view/agent/components/decision-panel-view.tsx"],
    "app/view/agent/hooks/useAgentQuestions.ts": ["app/view/agent/components/question-panel-view.tsx"],
    "app/view/browser/browser-signin-key.tsx": ["app/view/browser/browser-signin-key.tsx"],
    "app/view/browser/browser-signin-list.tsx": ["app/view/browser/browser-signin-list.tsx"],
    "app/view/browser/browser-signin-offer.tsx": ["app/view/browser/browser-signin-offer.tsx"],
    "app/view/agent/components/AgentSeatPanelState.ts": ["app/view/agent/components/AgentSeatPanel.tsx"],
    "app/view/agent/components/SeatRosterImport.tsx": ["app/view/agent/components/AgentSeatPanel.tsx"],
    "app/view/agent/failure/takeover.ts": ["app/view/agent/failure/failure-accessory.ts"],
    "app/view/agent/failure/open-in-other-pane.ts": ["app/view/agent/failure/failure-accessory.ts"],
    "app/view/drone/drone-template-picker.tsx": ["app/view/drone/drone-template-picker.tsx"],
    "app/view/swarm/swarm-model.ts": ["app/view/swarm/swarm-broadcast.tsx"],
    "app/view/tower/tower-model.ts": ["app/view/tower/tower-view.tsx"],
    "app/view/warden-jobs/jobs-model.ts": ["app/view/warden-jobs/job-template-approval.tsx"],
};

const READ = "reads state for a view; answers nothing for the person";
const OWN_VIEW = "the person's own control in a pane an agent's UI tool does not reach (not the agent's own pane)";
const AGENT_API = "an action an agent may take as itself through its own API (not a prompt)";
const SEND = "sends the person's typing or a system message into a conversation; no prompt";
const SETTING = "a setting the person changes in Settings or a pane an agent's UI tool does not reach";

/** Files that call a person-only command and are not prompts, with why. */
const CONTROLS: Record<string, string> = {
    "app/block/sandboxed-widget-host.tsx": "opens a widget's own session; no prompt",
    "app/element/blockstats.tsx": READ,
    "app/statusbar/HistoryUsageIndicator.tsx": READ,
    "app/statusbar/LanPeerRow.tsx": READ,
    "app/statusbar/gpu-panel-state.ts": READ,
    "app/store/account-availability.ts": OWN_VIEW,
    "app/store/account-usage.ts": READ,
    "app/view/agent/agent-model.ts": "writes the agent's own config on launch",
    "app/view/agent/commands/dispatch.ts": SEND,
    "app/view/agent/commands/types.ts": SEND,
    "app/view/agent/defaults/instance-slug.ts": "writes the agent's own config on launch",
    "app/view/agent/flows/launch-flow.ts": SEND,
    "app/view/agent/flows/register-seeded-account.ts": "records a sign-in the person just completed",
    "app/view/agent/history/HistoryNavigator.tsx": READ,
    "app/view/agent/history/history-navigation.ts": READ,
    "app/view/agent/hooks/useAgentCommands.ts": SEND,
    "app/view/agent/hooks/usePtyWidth.ts": "resizes the person's own pane; no prompt",
    "app/view/agent/launch-agent-steps.ts": "writes the agent's own config on launch",
    "app/view/agent/memory-reinjection-controller.ts": SEND,
    "app/view/agent/offline/offline-input.ts": SEND,
    "app/view/agent/stream-memory-reinjection.ts": SEND,
    "app/view/audit/audit-history-target.ts": READ,
    "app/view/audit/audit-list.ts": READ,
    "app/view/browser/browser-profiles.ts": SETTING,
    "app/view/audit/conversations/history-agents.tsx": READ,
    "app/view/audit/conversations/history-sessions.ts": READ,
    "app/view/audit/storage/history-storage.tsx": OWN_VIEW,
    "app/view/kanban/kanban-model.ts": OWN_VIEW,
    "app/view/settings/sections/pinned-senders.tsx": SETTING,
    "app/view/skill/skill-app-run-test.tsx": OWN_VIEW,
    "app/view/swarm/swarm-row-registration.tsx": AGENT_API,
    "app/view/term/termagent.ts": AGENT_API,
};

/** The frontend's own source files: not tests, type files or the generated RPC stubs. */
function sources(dir: string, out: string[] = []): string[] {
    for (const name of readdirSync(dir)) {
        const p = join(dir, name);
        if (statSync(p).isDirectory()) {
            if (name === "node_modules" || name === "rpc-api" || name === "types" || name === "__snapshots__") continue;
            sources(p, out);
        } else if (/\.tsx?$/.test(name) && !/\.test\.tsx?$/.test(name) && !name.endsWith(".d.ts")) out.push(p);
    }
    return out;
}

/** Each person-only command's `RpcApi` method, read from the generated stubs and the hand-written ones. */
function methodsByCommand(): Map<string, string> {
    const map = new Map<string, string>();
    for (const file of ["app/store/rpc-api/generated.ts", "app/store/rpc-api/index.ts"]) {
        const text = readFileSync(join(FRONTEND, file), "utf8");
        for (const m of text.matchAll(/(\w+Command)\s*\([^)]*\)[^{]*\{\s*return\s+\w+\.rpcCall\(\s*"([^"]+)"/g)) map.set(m[2], m[1]);
    }
    return map;
}

describe("prompts that speak for the person", () => {
    const methods = methodsByCommand();

    it("finds an RpcApi method for every command only the person's window may send", () => {
        const missing = [...OPERATOR_COMMANDS].filter((c) => !methods.has(c));
        expect(missing, `no RpcApi method found for ${missing.join(", ")}: this test reads them from rpc-api/generated.ts and index.ts`).toEqual([]);
    });

    it("has every caller of such a command on one of the two lists, so none is added unmarked", () => {
        const names = [...OPERATOR_COMMANDS].map((c) => methods.get(c)).filter((m): m is string => !!m);
        const unclassified: string[] = [];
        for (const file of sources(FRONTEND)) {
            const rel = relative(FRONTEND, file).split("\\").join("/");
            const text = readFileSync(file, "utf8");
            const called = names.filter((n) => new RegExp(`\\b${n}\\b`).test(text));
            if (called.length > 0 && !(rel in PROMPTS) && !(rel in CONTROLS)) {
                unclassified.push(
                    `${rel} calls ${called.join(", ")}, a command only the person's window may send. If it draws a prompt whose answer is the person's own (an answer, an approval, consent), ` +
                        `mark the prompt's root with data-operator-only (a Modal's operatorOnly prop, a pane request's operatorOnly: true) and add the file to PROMPTS in ${relative(FRONTEND, fileURLToPath(import.meta.url))}; ` +
                        `if it is an ordinary control or a read, add it to CONTROLS there with the reason.`
                );
            }
        }
        expect(unclassified).toEqual([]);
    });

    it("lists no file that is gone or no longer calls such a command", () => {
        const names = [...OPERATOR_COMMANDS].map((c) => methods.get(c)).filter((m): m is string => !!m);
        const stale = [...Object.keys(PROMPTS), ...Object.keys(CONTROLS)].filter((rel) => {
            let text: string;
            try {
                text = readFileSync(join(FRONTEND, rel), "utf8");
            } catch {
                return true;
            }
            return !names.some((n) => new RegExp(`\\b${n}\\b`).test(text));
        });
        expect(stale, "remove these from PROMPTS or CONTROLS: they are gone or no longer call a person-only command").toEqual([]);
    });

    it("carries the marker in every file that draws such a prompt", () => {
        const unmarked: string[] = [];
        for (const [caller, drawn] of Object.entries(PROMPTS)) {
            for (const file of drawn) {
                const text = readFileSync(join(FRONTEND, file), "utf8");
                if (!/data-operator-only|operatorOnly/.test(text)) {
                    unmarked.push(`${file} draws a prompt answered through ${caller} and has no data-operator-only: add it to the prompt's root element (a Modal's operatorOnly prop, a pane request's operatorOnly: true)`);
                }
            }
        }
        expect(unmarked).toEqual([]);
    });
});

/**
 * The host-relayed answers (`getApi().approvals.*`) are the person's too: a source file that calls one must be a
 * prompt that carries the marker, or a control that is listed with why. The same goes for a control that acts for the
 * person without a command (the hosted workspace's stop panel).
 */
const RELAYED_PROMPTS: Record<string, string[]> = {
    "app/block/desktop-requests.tsx": ["app/block/desktop-requests.tsx"],
    "app/block/upgrade-requests.tsx": ["app/block/upgrade-requests.tsx"],
    "app/block/widget-install-requests.tsx": ["app/block/widget-install-requests.tsx"],
    "app/view/browser/browser-model.ts": ["app/view/browser/browser-attention.tsx"],
    "app/store/app-update.ts": ["app/statusbar/RestartToUpdatePopover.tsx", "app/statusbar/MaintenanceSection.tsx"],
    "app/statusbar/maintenance-store.ts": ["app/statusbar/MaintenanceSection.tsx"],
};

const RELAYED_CONTROLS: Record<string, string> = {
    "app/store/rpc-util.ts": "proves the window's own connection; no prompt",
    "app/view/accounts/google-services.ts": OWN_VIEW,
    "app/view/agent/components/DesktopDrivingBar.tsx": "ends the agent's own desktop grants",
    "app/view/credential-approval/CredentialApprovalWindow.tsx": OWN_VIEW,
    "app/view/settings/sections/widgets-section-parts.tsx": SETTING,
    "app/view/settings/sections/widgets-section.tsx": SETTING,
    "app/view/ssh-approval/SshApprovalWindow.tsx": OWN_VIEW,
    "app/view/vault/vault-store.ts": OWN_VIEW,
};

/** Controls that act for the person with no command or relay of their own, and so are named here. */
const MARKED_CONTROLS = ["app/statusbar/WorkspaceStopStatus.tsx"];

const RELAY_CALL = /\bapprovals\??\.[A-Za-z]+/;

describe("host-relayed answers that speak for the person", () => {
    it("has every caller on one of the two lists, so none is added unmarked", () => {
        const unclassified: string[] = [];
        for (const file of sources(FRONTEND)) {
            const rel = relative(FRONTEND, file).split("\\").join("/");
            if (rel.startsWith("app/host/")) continue;
            if (!RELAY_CALL.test(readFileSync(file, "utf8"))) continue;
            if (!(rel in RELAYED_PROMPTS) && !(rel in RELAYED_CONTROLS)) {
                unclassified.push(
                    `${rel} calls a host-relayed answer (approvals.*). If it draws a prompt or panel whose answer is the person's own, mark its root with data-operator-only ` +
                        `(a Modal's or AnchoredPopover's operatorOnly prop, a pane request's operatorOnly: true) and add the file to RELAYED_PROMPTS in ${relative(FRONTEND, fileURLToPath(import.meta.url))}; ` +
                        `otherwise add it to RELAYED_CONTROLS there with the reason.`
                );
            }
        }
        expect(unclassified).toEqual([]);
    });

    it("lists no file that is gone or no longer calls one", () => {
        const stale = [...Object.keys(RELAYED_PROMPTS), ...Object.keys(RELAYED_CONTROLS)].filter((rel) => {
            try {
                return !RELAY_CALL.test(readFileSync(join(FRONTEND, rel), "utf8"));
            } catch {
                return true;
            }
        });
        expect(stale, "remove these from RELAYED_PROMPTS or RELAYED_CONTROLS").toEqual([]);
    });

    it("carries the marker in every file that draws such a prompt or panel", () => {
        const unmarked: string[] = [];
        const drawn = [...Object.values(RELAYED_PROMPTS).flat(), ...MARKED_CONTROLS];
        for (const file of new Set(drawn)) {
            if (!/data-operator-only|operatorOnly/.test(readFileSync(join(FRONTEND, file), "utf8"))) {
                unmarked.push(`${file} draws controls that act for the person and has no data-operator-only`);
            }
        }
        expect(unmarked).toEqual([]);
    });
});

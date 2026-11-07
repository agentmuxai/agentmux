// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { type JSX } from "solid-js";

import { DisplayGroup, EchoGroup, KeysGroup, ShellAndTextGroup, SshGroup, WatchdogGroup } from "./terminal-section-parts";

export { TERMINAL_SETTINGS } from "./terminal-settings-index";

// ── Section: Terminal ─────────────────────────────────────────────────────────

export function TerminalSection(): JSX.Element {
    return (
        <div class="settings-section-body">
            <ShellAndTextGroup />
            <KeysGroup />
            <SshGroup />
            <DisplayGroup />
            <EchoGroup />
            <WatchdogGroup />
        </div>
    );
}

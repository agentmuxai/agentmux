// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The create panel's Tools step: each missing tool with its install link and the inline installer. */

import { For, Show, type JSX } from "solid-js";

import { Button } from "@/app/element/ui";
import { SystemToolInstallInline } from "@/app/view/toolchain/SystemToolInstallInline";
import { open, type MissingPrereq } from "./create-panel-model";
import type { PanelParts } from "./create-panel-view";
import { SYSTEM_INSTALLABLE_IDS } from "./use-tool-install-info";

function ToolRow(props: { p: PanelParts; req: MissingPrereq }): JSX.Element {
    const { tools, info } = props.p;
    const { resolvedInstalls, unavailable, expanded, toggle, markUnavailable } = info;
    const req = props.req;
    return (
        <li class="agent-create-tool-row" classList={{ "is-ok": tools.installedPendingRestart().has(req.tool) }}>
            <Show
                when={!tools.installedPendingRestart().has(req.tool)}
                fallback={
                    <div class="agent-create-tool-info">
                        <div class="agent-create-tool-tool">
                            {req.label}
                            <span class="agent-create-tool-tool-status agent-create-tool-tool-status-ok">
                                {" "}— installed, restart AgentMux to use it
                            </span>
                        </div>
                    </div>
                }
            >
                <div class="agent-create-tool-info">
                    <div class="agent-create-tool-tool">
                        {req.label}
                        <span class="agent-create-tool-tool-status"> — not found</span>
                    </div>
                    <a
                        class="agent-create-tool-link"
                        href={req.installUrl}
                        onClick={(e) => {
                            e.preventDefault();
                            open(req.installUrl);
                        }}
                    >
                        {req.installLinkText}
                        <span aria-hidden="true"> ↗</span>
                    </a>
                    <Show when={SYSTEM_INSTALLABLE_IDS.has(req.tool) && !unavailable().has(req.tool)}>
                        <Button tone="quiet" onClick={() => toggle(req.tool)}>
                            {(() => {
                                const v = resolvedInstalls().get(req.tool)?.resolvedVersion;
                                return v ? `Install v${v} now ↓` : "or install it now ↓";
                            })()}
                        </Button>
                        <Show when={expanded().has(req.tool)}>
                            <SystemToolInstallInline
                                toolId={req.tool}
                                resolvedInfo={resolvedInstalls().get(req.tool)}
                                onInstalled={() => {
                                    tools.setInstalledPendingRestart((prev) => new Set(prev).add(req.tool));
                                    void tools.checkTools(true);
                                }}
                                onUnavailable={() => markUnavailable(req.tool)}
                            />
                        </Show>
                    </Show>
                </div>
            </Show>
        </li>
    );
}

export function ToolsStep(props: { p: PanelParts }): JSX.Element {
    const { s, tools } = props.p;
    return (
        <Show when={tools.missing() !== null && !s.runtime.deciding()} fallback={<p class="agent-create-panel-text">Checking what {s.props.template.name} needs…</p>}>
            <div class="agent-create-tools" data-testid="agent-create-tools">
                <p class="agent-create-panel-text">
                    {s.props.template.name} needs tools that aren't on your PATH. Install them, then check
                    again.
                </p>
                <ul class="agent-create-tool-list">
                    <For each={tools.missing() ?? []}>{(req) => <ToolRow p={props.p} req={req} />}</For>
                </ul>
                <p class="agent-create-tool-hint">
                    Installed somewhere AgentMux can't see? Continue anyway: your CLI might still find it.
                </p>
                <div class="agent-create-actions">
                    <Button tone="neutral" onClick={() => s.props.onClose()}>
                        Cancel
                    </Button>
                    <Button tone="neutral" onClick={() => void tools.checkTools(true)}>
                        Check again
                    </Button>
                    <Button tone="accent" onClick={() => tools.setSkipTools(true)} data-testid="agent-create-tools-continue">
                        Continue anyway
                    </Button>
                </div>
            </div>
        </Show>
    );
}

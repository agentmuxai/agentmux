// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The create panel's frame: the title and step trail, the container offer, and the current step. */

import { For, Match, Show, Switch, type JSX } from "solid-js";

import { Button } from "@/app/element/ui";
import { AgentCreateForm } from "./AgentCreateForm";
import { FreeStartGuide } from "./FreeStartGuide";
import type { InstallStep } from "./create-panel-install";
import type { FreeGuide } from "./create-panel-guide";
import { NpmInstall, VendorInstall, VendorRun } from "./create-panel-install-view";
import { STEP_LABEL } from "./create-panel-model";
import type { PanelState } from "./create-panel-state";
import type { StepFlow } from "./create-panel-steps";
import type { ToolsCheck } from "./create-panel-tools";
import { ToolsStep } from "./create-panel-tools-view";
import type { useToolInstallInfo } from "./use-tool-install-info";

/** Everything the panel's parts read: the state and the hook groups that act on it. */
export interface PanelParts {
    s: PanelState;
    free: FreeGuide;
    tools: ToolsCheck;
    install: InstallStep;
    flow: StepFlow;
    info: ReturnType<typeof useToolInstallInfo>;
}

function PanelHeader(props: { p: PanelParts }): JSX.Element {
    const { s, flow } = props.p;
    return (
        <header class="agent-create-panel-header">
            <h3 class="agent-create-panel-title">New {s.props.template.name} agent</h3>
            <Show when={flow.steps().length > 1}>
                <ol class="agent-create-steps" data-testid="agent-create-steps">
                    <For each={flow.steps()}>
                        {(st) => (
                            <li class="agent-create-step" classList={{ "agent-create-step--current": st === flow.step() }} aria-current={st === flow.step() ? "step" : undefined}>
                                {STEP_LABEL[st]}
                            </li>
                        )}
                    </For>
                </ol>
            </Show>
        </header>
    );
}

function CreateStep(props: { p: PanelParts }): JSX.Element {
    const { s, free } = props.p;
    const { props: panelProps } = s;
    return (
        <>
            {/* A provider with a free sign-in route, for someone with no
                account for it yet: the link to sign up, then the steps. */}
            <Show when={free.showFreeGuide()}>
                <FreeStartGuide provider={s.providerId() ?? panelProps.template.provider} name={s.name()} />
            </Show>
            <AgentCreateForm
                template={panelProps.template}
                draft={panelProps.draft}
                runtime={s.runtime}
                onCreated={panelProps.onCreated}
                onCancel={() => panelProps.onClose()}
                onBusyChange={panelProps.onBusyChange}
            />
        </>
    );
}

export function PanelView(props: { p: PanelParts }): JSX.Element {
    const { s, install, flow } = props.p;
    return (
        <section class="agent-create-panel" aria-label={`New ${s.props.template.name} agent`} data-testid="agent-create-panel">
            <PanelHeader p={props.p} />

            <Show when={flow.offerContainer()}>
                <p class="agent-create-panel-text" data-testid="agent-create-container-offer">
                    A container agent has nothing to install on this computer.{" "}
                    <Button tone="quiet" onClick={() => s.runtime.pickRuntime("container")} data-testid="agent-create-use-container">
                        Run it in a container instead
                    </Button>
                </p>
            </Show>

            <Switch>
                <Match when={flow.step() === "tools"}>
                    <ToolsStep p={props.p} />
                </Match>
                <Match when={flow.step() === "install" && s.userInstalled() && install.session()}>
                    {(sess) => <VendorRun p={props.p} session={sess} />}
                </Match>
                <Match when={flow.step() === "install" && s.userInstalled()}>
                    <VendorInstall p={props.p} />
                </Match>
                <Match when={flow.step() === "install" && install.session()}>
                    {(sess) => <NpmInstall p={props.p} session={sess} />}
                </Match>
                <Match when={flow.step() === "create"}>
                    <CreateStep p={props.p} />
                </Match>
            </Switch>
        </section>
    );
}

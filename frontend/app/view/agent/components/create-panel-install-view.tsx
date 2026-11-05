// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The create panel's Install step: the vendor's own installer, or the run AgentMux makes for an npm-managed CLI. */

import { Match, Show, Switch, type Accessor, type JSX } from "solid-js";

import { Button } from "@/app/element/ui";
import { InstallProgress } from "@/element/install/InstallProgress";
import { writeText as clipboardWriteText } from "@/util/clipboard";
import { open } from "./create-panel-model";
import type { PanelParts } from "./create-panel-view";

export function VendorInstall(props: { p: PanelParts }): JSX.Element {
    const { s, install } = props.p;
    const { name, provider } = s;
    return (
        <div class="agent-create-install" data-testid="agent-create-vendor-install">
            <p class="agent-create-panel-text">
                AgentMux doesn't install {name()}: install it with its own installer, which puts it on your
                PATH, outside AgentMux's isolation. Then check again.
            </p>
            <Show when={install.vendorCommand()}>
                <div class="agent-create-command">
                    <code data-testid="agent-create-vendor-command">{install.vendorCommand()}</code>
                    <Button tone="quiet" onClick={() => void clipboardWriteText(install.vendorCommand())} title="Copy">
                        <i class="fa-solid fa-copy" aria-hidden="true" /> Copy
                    </Button>
                </div>
            </Show>
            <Show when={provider()?.docsUrl}>
                {(url) => (
                    <a
                        class="agent-create-tool-link"
                        href={url()}
                        onClick={(e) => {
                            e.preventDefault();
                            open(url());
                        }}
                    >
                        {name()}'s install instructions<span aria-hidden="true"> ↗</span>
                    </a>
                )}
            </Show>
            <div class="agent-create-actions">
                <Button tone="neutral" onClick={() => s.props.onClose()}>
                    Cancel
                </Button>
                <Button tone="accent" busy={install.checkingAgain()} onClick={() => void install.checkAgain()} data-testid="agent-create-check-again">
                    Check again
                </Button>
            </div>
        </div>
    );
}

type Session = NonNullable<ReturnType<PanelParts["install"]["session"]>>;

export function NpmInstall(props: { p: PanelParts; session: Accessor<Session> }): JSX.Element {
    const { s, install } = props.p;
    const sess = props.session;
    return (
        <div class="agent-create-install" data-testid="agent-create-npm-install">
            <p class="agent-create-panel-text">
                {s.name()} isn't installed yet. AgentMux installs its own copy, kept apart from any you
                installed yourself. It needs an internet connection, and keeps going if you close this.
            </p>
            <InstallProgress session={sess()} kind="provider-cli" />
            <div class="agent-create-actions">
                <Switch>
                    <Match when={sess().state() === "idle"}>
                        <Button tone="neutral" onClick={() => s.props.onClose()}>
                            Cancel
                        </Button>
                        <Button tone="accent" onClick={() => void sess().start()} data-testid="agent-create-install-start">
                            Install now
                        </Button>
                    </Match>
                    <Match when={sess().state() === "running"}>
                        <Button tone="neutral" onClick={() => void sess().cancel()} data-testid="agent-create-install-cancel">
                            Cancel install
                        </Button>
                    </Match>
                    <Match when={sess().state() === "failed"}>
                        <Button tone="neutral" onClick={() => s.props.onClose()}>
                            Close
                        </Button>
                        <Button tone="accent" onClick={() => void sess().start()}>
                            Retry
                        </Button>
                    </Match>
                    <Match when={sess().state() === "done"}>
                        <Button tone="neutral" onClick={() => s.props.onClose()}>
                            Close
                        </Button>
                        <Button
                            tone="accent"
                            onClick={() => {
                                install.setInstallDone(true);
                                void s.props.recheckInstalled();
                            }}
                            data-testid="agent-create-install-continue"
                        >
                            Continue
                        </Button>
                    </Match>
                </Switch>
            </div>
        </div>
    );
}

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The create panel's Install step: the vendor's own installer, or the run AgentMux makes for an npm-managed CLI. */

import { createEffect, on, Match, Show, Switch, type Accessor, type JSX } from "solid-js";

import { Button } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
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
                {t("createPanel.install.vendor", { name: name() })}
            </p>
            <Show when={install.vendorCommand()}>
                <div class="agent-create-command">
                    <code data-testid="agent-create-vendor-command">{install.vendorCommand()}</code>
                    <Button tone="quiet" onClick={() => void clipboardWriteText(install.vendorCommand())} title={t("createPanel.install.copy")}>
                        <i class="fa-solid fa-copy" aria-hidden="true" /> {t("createPanel.install.copy")}
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
                        {t("createPanel.install.instructions", { name: name() })}
                        <span aria-hidden="true"> ↗</span>
                    </a>
                )}
            </Show>
            <div class="agent-create-actions">
                <Button tone="neutral" onClick={() => s.props.onClose()}>
                    {t("createPanel.install.cancel")}
                </Button>
                <Button tone="accent" busy={install.checkingAgain()} onClick={() => install.checkAgain()} data-testid="agent-create-check-again">
                    {t("createPanel.install.checkAgain")}
                </Button>
            </div>
        </div>
    );
}

type Session = NonNullable<ReturnType<PanelParts["install"]["session"]>>;

/**
 * Bring an element into view without moving anything that is already in view. The panel opens inside a list that
 * scrolls, and in a small pane its steps are below the fold; the run is what the person is waiting on.
 */
export function keepInView(el: HTMLElement | undefined): void {
    el?.scrollIntoView?.({ block: "nearest", inline: "nearest" });
}

const PERSON_SCROLLS = ["wheel", "touchmove", "keydown", "pointerdown"] as const;
/** How long a panel that opened at once keeps itself in view while what is above it loads. */
const SETTLE_MS = 10_000;

/** The nearest ancestor that scrolls: the list the panel is in. */
function scrollerOf(el: HTMLElement): HTMLElement | undefined {
    for (let up = el.parentElement; up; up = up.parentElement) {
        if (/auto|scroll|overlay/.test(getComputedStyle(up).overflowY)) return up;
    }
    return undefined;
}

/**
 * [`keepInView`], and again each time what is around the panel changes: the cards and sections above a panel that
 * opens at once can still be loading, and an icon that arrives or text that wraps again pushes it down with no change
 * to the tree, so it also looks again when the size of the panel, of the list's content or of the list changes. It
 * stops at the first thing the person does, and after ten seconds.
 */
export function keepInViewWhileItSettles(el: HTMLElement | undefined): void {
    const parent = el?.parentElement;
    if (!el || !parent) return;
    keepInView(el);
    const doc = el.ownerDocument;
    const again = () => keepInView(el);
    const watch = new MutationObserver(again);
    const resized = typeof ResizeObserver === "undefined" ? undefined : new ResizeObserver(again);
    let timer: ReturnType<typeof setTimeout> | undefined;
    const stop = () => {
        watch.disconnect();
        resized?.disconnect();
        clearTimeout(timer);
        for (const type of PERSON_SCROLLS) doc.removeEventListener(type, stop);
    };
    watch.observe(parent, { childList: true, subtree: true });
    for (const target of [el, parent, scrollerOf(el)]) if (target) resized?.observe(target);
    timer = setTimeout(stop, SETTLE_MS);
    for (const type of PERSON_SCROLLS) doc.addEventListener(type, stop, { passive: true });
}

export function NpmInstall(props: { p: PanelParts; session: Accessor<Session> }): JSX.Element {
    const { s, install } = props.p;
    const sess = props.session;
    let root: HTMLDivElement | undefined;
    // The run starts, or ends badly: show its steps. (Its end in success moves the panel to the next step, which
    // keeps itself in view: PanelView.)
    createEffect(
        on(
            () => sess().state(),
            (state) => {
                if (state === "running" || state === "failed") keepInView(root);
            },
            { defer: true },
        ),
    );
    return (
        <div ref={root} class="agent-create-install" data-testid="agent-create-npm-install">
            <p class="agent-create-panel-text">
                {t("createPanel.install.npm", { name: s.name() })}
            </p>
            <InstallProgress session={sess()} kind="provider-cli" />
            <div class="agent-create-actions">
                <Switch>
                    <Match when={sess().state() === "idle"}>
                        <Button tone="neutral" onClick={() => s.props.onClose()}>
                            {t("createPanel.install.cancel")}
                        </Button>
                        <Button tone="accent" onClick={() => sess().start()} data-testid="agent-create-install-start">
                            {t("createPanel.install.now")}
                        </Button>
                    </Match>
                    <Match when={sess().state() === "running"}>
                        <Button tone="neutral" onClick={() => sess().cancel()} data-testid="agent-create-install-cancel">
                            {t("createPanel.install.cancelInstall")}
                        </Button>
                    </Match>
                    <Match when={sess().state() === "failed"}>
                        <Button tone="neutral" onClick={() => s.props.onClose()}>
                            {t("createPanel.install.close")}
                        </Button>
                        <Button tone="accent" onClick={() => sess().start()}>
                            {t("createPanel.install.retry")}
                        </Button>
                    </Match>
                    <Match when={sess().state() === "done"}>
                        <Button tone="neutral" onClick={() => s.props.onClose()}>
                            {t("createPanel.install.close")}
                        </Button>
                        <Button
                            tone="accent"
                            onClick={() => {
                                install.setInstallDone(true);
                                void s.props.recheckInstalled();
                            }}
                            data-testid="agent-create-install-continue"
                        >
                            {t("createPanel.install.continue")}
                        </Button>
                    </Match>
                </Switch>
            </div>
        </div>
    );
}

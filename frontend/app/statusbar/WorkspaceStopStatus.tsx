// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { AnchoredPopover } from "@/app/element/anchored-popover";
import { Button, Notice } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import { clockTime, countdown, startWorkspaceStop, workspaceStop } from "@/app/store/workspace-stop";
import { keepRunning, stopNow, type StopActionResult } from "@/app/store/workspace-stop-actions";
import { createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";

const POPOVER_WIDTH = 300;

/**
 * A hosted workspace is going to stop: how long is left and why, so nobody is surprised by it
 * (the workspaces-in-the-Console spec, sections 6.3 and 6.4). Only AgentMux Web has a host to tell
 * srv, so a desktop never shows it. The data is saved when it stops, and the workspace is started again from the Console.
 * Clicking it opens a panel with the two choices the spec names: Keep running, which cancels the stop, and Stop now, which ends
 * the wait. Either is one request to the workspace's own server; when the stop is gone the chip and the panel go with it.
 */
const WorkspaceStopStatus = (): JSX.Element => {
    onMount(startWorkspaceStop);
    const [now, setNow] = createSignal(Date.now());
    onMount(() => {
        const timer = setInterval(() => setNow(Date.now()), 1000);
        onCleanup(() => clearInterval(timer));
    });
    const [open, setOpen] = createSignal(false);
    const [problem, setProblem] = createSignal("");
    let chip: HTMLDivElement | undefined;

    const toggle = () => {
        setProblem("");
        setOpen(!open());
    };
    /** Runs one of the two choices; the panel stays open with the reason when it did not work. */
    const choose = async (action: () => Promise<StopActionResult>) => {
        setProblem("");
        const r = await action();
        if (r.ok) setOpen(false);
        else setProblem(r.message ?? "");
    };

    return (
        <Show when={workspaceStop()}>
            {(stop) => {
                const tip = () => t("workspaceStop.tip", { time: clockTime(stop().stopAtMs), reason: stop().reason });
                return (
                    <>
                        <div
                            ref={(el) => {
                                chip = el;
                            }}
                            class="status-bar-item clickable"
                            role="button"
                            tabindex="0"
                            data-tip={tip()}
                            aria-label={tip()}
                            aria-haspopup="dialog"
                            aria-expanded={open()}
                            onClick={toggle}
                            onKeyDown={(e) => {
                                if (e.key === "Enter" || e.key === " ") {
                                    e.preventDefault();
                                    toggle();
                                }
                            }}
                        >
                            <i class="fa-solid fa-power-off status-icon" style={{ color: "var(--warning-text-color)" }} />
                            <span style={{ color: "var(--warning-text-color)" }}>{t("workspaceStop.chip", { left: countdown(stop().stopAtMs, now()) })}</span>
                        </div>
                        <Show when={open()}>
                            <AnchoredPopover
                                anchor={chip}
                                placement="top-end"
                                onDismiss={() => setOpen(false)}
                                class="status-bar-popover"
                                role="dialog"
                                aria-label={t("workspaceStop.open")}
                                style={{ width: `${POPOVER_WIDTH}px` }}
                            >
                                <p>{tip()}</p>
                                <Show when={problem()}>
                                    <Notice tone="danger">{problem()}</Notice>
                                </Show>
                                <div class="status-bar-popover-row">
                                    <Button tone="accent" onClick={() => choose(keepRunning)}>
                                        {t("workspaceStop.keep")}
                                    </Button>
                                    <Button tone="danger" onClick={() => choose(stopNow)}>
                                        {t("workspaceStop.stopNow")}
                                    </Button>
                                </div>
                            </AnchoredPopover>
                        </Show>
                    </>
                );
            }}
        </Show>
    );
};

WorkspaceStopStatus.displayName = "WorkspaceStopStatus";

export { WorkspaceStopStatus };

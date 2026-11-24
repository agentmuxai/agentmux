// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The driven-by strip's markup in parts (driven-by-strip.tsx): the strip while the agent asks, the red strip while
 * Auto-approve is on, the buttons they share, the confirmation, and the list of what the agent did. Each reads the
 * strip's state (driven-by-strip-state.ts).
 */

import { Button } from "@/app/element/ui";
import { t as tr } from "@/app/i18n/i18n";
import { Rich } from "@/app/i18n/rich";
import { pausedLabel, browserPauseAll } from "@/app/store/browser-pause-all";
import { tip } from "@/app/element/tip";
import { agentDisplayName } from "@/app/store/agent-names";
import { For, Show, type JSX } from "solid-js";
import { actionFailed, actionKind, actionTime, shortUrl } from "./action-log";
import type { BrowserViewModel } from "./browser-model";
import { BYPASS_MINUTES, leftLabel, minutesLabel, powersNote, siteLabel } from "./bypass";
import type { DrivenByProps, DrivenByState } from "./driven-by-strip-state";

type Part = { t: DrivenByState; p: DrivenByProps };

/** The Log and Pause buttons, in both strips. */
function StripControls(props: Part): JSX.Element {
    const { t, p } = props;
    const model = t.model;
    return (
        <>
            <Button
                density="compact"
                class="browser-actions-log-button"
                pressed={t.showLog()}
                title={tr("browser.driven.logTitle", { agent: p.agent })}
                onClick={() => t.setShowLog(!t.showLog())}
            >
                {tr("browser.driven.log", { n: model.actionsAtom().count })}
            </Button>
            <Button
                density="compact"
                class="browser-pause-button"
                pressed={model.pausedAtom()}
                title={
                    model.pausedAtom()
                        ? t.everyPane()
                            ? tr("browser.driven.resumeEveryPane", { label: pausedLabel(browserPauseAll()) })
                            : tr("browser.driven.resumeHere", { agent: p.agent })
                        : tr("browser.driven.pauseTitle", { agent: p.agent })
                }
                onClick={t.togglePause}
            >
                {model.pausedAtom() ? tr("browser.driven.resume") : tr("browser.driven.pause")}
            </Button>
        </>
    );
}

/** Why Pause or Resume failed. */
function PauseErrorLine(props: Part): JSX.Element {
    const t = props.t;
    return (
        <Show when={t.pauseError()}>
            <span class="browser-bypass-strip-error" role="alert">
                {t.pauseError()}
            </span>
        </Show>
    );
}

/** The strip while the agent drives and asks: who, the sites it is limited to, Auto-approve and Take over. */
export function PlainStrip(props: Part): JSX.Element {
    const { t, p } = props;
    const model = t.model;
    return (
        <div class="browser-driven-by" classList={{ "browser-driven-by--held": model.pausedAtom() }} role="status">
            <i class={model.pausedAtom() ? "fa-solid fa-pause" : "fa-solid fa-robot"} aria-hidden="true" />
            <div class="browser-driven-by-message">
                <Show
                    when={!model.pausedAtom()}
                    fallback={
                        <span>
                            <Rich k={t.everyPane() ? "browser.driven.pausedEvery" : "browser.driven.pausedHere"} params={{ agent: p.agent }} />
                        </span>
                    }
                >
                    <span>
                        <Rich k="browser.driven.drivenBy" params={{ agent: p.agent }} />
                        <Show when={model.tookControlAtom()}>
                            {" "}
                            <span class="browser-took-control">{tr("browser.driven.tookControl")}</span>
                        </Show>
                    </span>
                </Show>
                <Show when={model.allowedOriginsAtom().length > 0}>
                    <span
                        class="browser-limited-to"
                        {...tip(tr("browser.driven.limitedTip", { sites: model.allowedOriginsAtom().join(", ") }))}
                    >
                        {tr("browser.driven.limitedTo", { count: model.allowedOriginsAtom().length })}
                    </span>
                </Show>
                <Show when={model.bypassEndedAtom()}>
                    {(why) => <span class="browser-bypass-ended">{tr("browser.driven.bypassEnded", { why: why() })}</span>}
                </Show>
                <PauseErrorLine {...props} />
            </div>
            <div class="browser-driven-by-actions">
                <StripControls {...props} />
                <Button
                    density="compact"
                    tone="danger"
                    class="browser-bypass-button"
                    pressed={t.confirming()}
                    title={tr("browser.driven.autoApproveTitle", { agent: p.agent })}
                    onClick={() => {
                        t.setError(undefined);
                        t.setConfirming(!t.confirming());
                    }}
                >
                    {tr("browser.driven.autoApprove")}
                </Button>
                <Button
                    density="compact"
                    class="browser-take-over"
                    title={tr("browser.driven.takeOverTitle")}
                    onClick={() => model.takeOver().catch(() => {})}
                >
                    {tr("browser.driven.takeOver")}
                </Button>
            </div>
        </div>
    );
}

/** The red strip while Auto-approve is on: the sites, the time left, what changes when paused, Stop and Take over. */
export function BypassStrip(props: Part & { grant: () => BrowserBypass }): JSX.Element {
    const { t, grant: b } = props;
    const model = t.model;
    return (
        <div
            class="browser-driven-by browser-driven-by--bypass"
            classList={{
                "browser-driven-by--paused": model.bypassPaused() || model.pausedAtom(),
            }}
            role="status"
        >
            <i class={model.pausedAtom() ? "fa-solid fa-pause" : "fa-solid fa-bolt"} aria-hidden="true" />
            <div class="browser-driven-by-message">
                <Show
                    when={!model.pausedAtom()}
                    fallback={
                        <span>
                            <Rich
                                k={t.everyPane() ? "browser.driven.bypassPausedEvery" : "browser.driven.bypassPausedHere"}
                                params={{
                                    agent: agentDisplayName(b().agent),
                                    note: b().always
                                        ? tr("browser.driven.heldAlways")
                                        : tr("browser.driven.clockStopped", { minutes: model.bypassMinutesLeft() }),
                                }}
                            />
                        </span>
                    }
                >
                    <Show
                        when={!model.bypassPaused()}
                        fallback={
                            <span>
                                <Rich
                                    k="browser.driven.pageLeft"
                                    params={{ sites: b().origins.map(siteLabel).join(", "), agent: agentDisplayName(b().agent) }}
                                />
                            </span>
                        }
                    >
                        <span>
                            <Rich
                                k="browser.driven.bypassOn"
                                params={{
                                    agent: agentDisplayName(b().agent),
                                    sites: b().origins.map(siteLabel).join(", "),
                                    powers: powersNote(b().powers),
                                    left: leftLabel(b().always, model.bypassMinutesLeft()),
                                }}
                            />
                        </span>
                    </Show>
                </Show>
                <Show when={t.error()}>
                    <span class="browser-bypass-strip-error" role="alert">
                        {t.error()}
                    </span>
                </Show>
                <PauseErrorLine {...props} />
            </div>
            <div class="browser-driven-by-actions">
                <StripControls {...props} />
                <Button density="compact" tone="danger" class="browser-bypass-end" onClick={t.end}>
                    {tr("browser.driven.stopAutoApprove")}
                </Button>
                <Button
                    density="compact"
                    class="browser-take-over-inline"
                    title={tr("browser.driven.takeOverTitle")}
                    onClick={() => model.takeOver().catch(() => {})}
                >
                    {tr("browser.driven.takeOver")}
                </Button>
            </div>
        </div>
    );
}

/** The confirmation for Auto-approve: the sites, a duration, and why it failed. */
export function BypassConfirm(props: Part): JSX.Element {
    const { t, p } = props;
    return (
        <div class="browser-bypass-confirm" role="dialog" aria-label={tr("browser.driven.confirmLabel")}>
            <div class="browser-bypass-confirm-head">
                <i class="fa-solid fa-bolt" aria-hidden="true" />
                <span>
                    <Rich k="browser.driven.confirmHead" params={{ agent: p.agent }} />
                </span>
            </div>
            <Show
                when={t.sites().length > 0}
                fallback={<div class="browser-bypass-confirm-text">{tr("browser.driven.openPageFirst")}</div>}
            >
                <div class="browser-bypass-confirm-text">
                    <Rich k="browser.driven.confirmText" params={{ sites: t.sites().map(siteLabel).join(", "), agent: p.agent }} />
                </div>
                <div class="browser-bypass-confirm-minutes" role="radiogroup" aria-label={tr("browser.driven.forHowLong")}>
                    <For each={BYPASS_MINUTES}>
                        {(m) => (
                            <Button
                                density="compact"
                                pressed={t.minutes() === m}
                                role="radio"
                                aria-checked={t.minutes() === m}
                                onClick={() => t.setMinutes(m)}
                            >
                                {minutesLabel(m)}
                            </Button>
                        )}
                    </For>
                </div>
            </Show>
            <Show when={t.error()}>
                <div class="browser-bypass-confirm-error" role="alert">
                    {t.error()}
                </div>
            </Show>
            <div class="browser-bypass-confirm-actions">
                <Button
                    tone="danger"
                    density="compact"
                    busy={t.busy()}
                    disabled={t.sites().length === 0}
                    onClick={t.turnOn}
                >
                    {tr("browser.driven.turnOn")}
                </Button>
                <Button density="compact" onClick={() => t.setConfirming(false)}>
                    {tr("browser.driven.cancel")}
                </Button>
            </div>
        </div>
    );
}

/** The list of what the agent did, newest first, in the pane's flow. */
export function ActionLogPanel(props: { model: BrowserViewModel; agent: string; onClose: () => void }): JSX.Element {
    const newestFirst = () => [...props.model.actionsAtom().entries].reverse();
    return (
        <div class="browser-action-log" role="log" aria-label={tr("browser.driven.logTitle", { agent: props.agent })}>
            <div class="browser-action-log-head">
                <span>
                    <Rich k="browser.driven.logHead" params={{ agent: props.agent }} />
                    <Show when={props.model.actionsAtom().count > props.model.actionsAtom().entries.length}>
                        {" "}
                        {tr("browser.driven.logLatest", {
                            shown: props.model.actionsAtom().entries.length,
                            total: props.model.actionsAtom().count,
                        })}
                    </Show>
                </span>
                <Button density="compact" onClick={props.onClose}>
                    {tr("browser.driven.close")}
                </Button>
            </div>
            <Show when={newestFirst().length > 0} fallback={<div class="browser-action-log-empty">{tr("browser.driven.nothingYet")}</div>}>
                <ol class="browser-action-log-list">
                    <For each={newestFirst()}>
                        {(e) => (
                            <li
                                class="browser-action-log-row"
                                classList={{ "browser-action-log-row--failed": actionFailed(e.how) }}
                            >
                                <span class="browser-action-log-time">{actionTime(e.ts)}</span>
                                <span class="browser-action-log-kind">{actionKind(e.kind)}</span>
                                <span class="browser-action-log-target">
                                    {e.target ?? ""}
                                    <Show when={e.detail}>
                                        <span class="browser-action-log-detail">
                                            {e.target ? " · " : ""}
                                            {e.detail}
                                        </span>
                                    </Show>
                                </span>
                                <span class="browser-action-log-how">{e.how}</span>
                                <span class="browser-action-log-url" {...tip(e.url)}>
                                    {shortUrl(e.url)}
                                </span>
                            </li>
                        )}
                    </For>
                </ol>
            </Show>
        </div>
    );
}

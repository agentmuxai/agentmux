// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The request a browser pane puts over its page when an agent or the page waits on the person: its words for each kind, and its buttons. */

import { createSignal, For, Match, Show, Switch, type Accessor, type JSX } from "solid-js";
import { Button } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import { Rich } from "@/app/i18n/rich";
import { tip } from "@/app/element/tip";
import { agentDisplayName, followAgentNames } from "@/app/store/agent-names";
import { attentionLook, dialogMessage } from "./attention-kinds";
import { asksEachLine, hasEnterSpacePower, hasTerminalPower, hasUploadsPower, LATE_PROMPT_LIMIT, LINE_PREVIEW_CHARS, minutesLabel, siteLabel } from "./bypass";
import type { BrowserViewModel } from "./browser-model";

type Part = { attention: Accessor<BrowserAttention> };

/** The agent's name, or the words for an agent that has none. */
function whoName(a: BrowserAttention): string {
    return agentDisplayName(a.agent ?? "") || t("browser.attention.anAgent");
}

/** The sites a bypass names, as one list. */
function sitesOf(a: BrowserAttention): string {
    return (a.origins ?? []).map(siteLabel).join(", ");
}
/** A long terminal line is read in full ("Show all") before Run it works. */
type LineView = { open: Accessor<boolean>; setOpen: (open: boolean) => void };

export function BrowserAttentionBanner(props: { model: BrowserViewModel; attention: Accessor<BrowserAttention> }): JSX.Element {
    const a = props.attention;
    const [open, setOpen] = createSignal(false);
    followAgentNames();
    return (
        <div
            class="browser-attention"
            classList={{ "browser-attention--agent": attentionLook(a()).agent }}
            data-kind={a().kind}
            role="alertdialog"
            aria-live="assertive"
        >
            <AttentionHead attention={a} />
            <AttentionDetails attention={a} open={open} setOpen={setOpen} />
            <AttentionActions model={props.model} attention={a} open={open} setOpen={setOpen} />
        </div>
    );
}

/** The sentence a page's dialog starts with: asking to leave or asking something, and after whose action if an agent's. */
function dialogKey(a: BrowserAttention): string {
    const leave = a.dialogKind === "beforeunload";
    if (a.agent) return leave ? "browser.attention.dialogLeaveAfter" : "browser.attention.dialogAskAfter";
    return leave ? "browser.attention.dialogLeave" : "browser.attention.dialogAsk";
}

/** The icon and the sentence that says who wants what. */
function AttentionHead(props: Part): JSX.Element {
    const a = props.attention;
    return (
        <div class="browser-attention-head">
            <i class={`fa-solid ${attentionLook(a()).icon}`} aria-hidden="true" />
            <Switch
                fallback={
                    <span>
                        <Rich
                            k={
                                a().kind === "handoff"
                                    ? "browser.attention.needs"
                                    : a().kind === "bypass"
                                      ? "browser.attention.bypassAsk"
                                      : "browser.attention.wants"
                            }
                            params={{
                                agent: whoName(a()),
                                what: (a().kind === "handoff" || a().kind === "bypass" ? a().reason : a().what) ?? "",
                            }}
                        />
                    </span>
                }
            >
                <Match when={attentionLook(a()).text}>{(text) => <span>{text()}</span>}</Match>
                <Match when={a().kind === "line"}>
                    <span>
                        <Rich
                            k={a().reason ? "browser.attention.lineWhy" : "browser.attention.line"}
                            params={{ agent: whoName(a()), site: siteLabel(a().origin ?? ""), reason: a().reason ?? "" }}
                        />
                    </span>
                </Match>
                <Match when={a().kind === "take"}>
                    <span>
                        <Rich
                            k={a().reason ? "browser.attention.takeWhy" : "browser.attention.take"}
                            params={{ agent: whoName(a()), reason: a().reason ?? "" }}
                        />
                    </span>
                </Match>
                <Match when={a().kind === "navigation"}>
                    <span>
                        <Rich
                            k={a().popup ? "browser.attention.navigationPopup" : "browser.attention.navigationGo"}
                            params={{ origin: a().origin ?? "" }}
                        />
                    </span>
                </Match>
                <Match when={a().kind === "dialog"}>
                    <span>
                        <Rich
                            k={dialogKey(a())}
                            params={{
                                origin: a().origin || t("browser.attention.thisSite"),
                                agent: agentDisplayName(a().agent ?? ""),
                            }}
                        />
                    </span>
                </Match>
            </Switch>
        </div>
    );
}

/** What the request is about, in the lines under the head that its kind has. */
function AttentionDetails(props: Part & LineView): JSX.Element {
    const a = props.attention;
    return (
        <>
            <Show when={a().kind === "line"}>
                <LineDetails attention={a} open={props.open} setOpen={props.setOpen} />
            </Show>
            <Show when={a().kind === "approval" && a().description}>
                <div class="browser-attention-target browser-attention-description">
                    {(a().description ?? "").length > 200
                        ? `${(a().description ?? "").slice(0, 200)}…`
                        : a().description}
                </div>
            </Show>
            <Show when={a().kind === "approval" && a().why}>
                <div class="browser-attention-target">{t("browser.attention.askedBecause", { why: a().why ?? "" })}</div>
            </Show>
            {/* The page's own words, as text in a box of their own, so
                they can't pass for AgentMux's (dialogs spec §8.1). */}
            <Show when={a().kind === "dialog" && dialogMessage(a().message)}>
                <blockquote class="browser-attention-page-text">{dialogMessage(a().message)}</blockquote>
            </Show>
            <Show when={attentionLook(a()).showUrl && a().url}>
                <div class="browser-attention-target" {...tip(a().url)}>
                    {a().url}
                </div>
            </Show>
            <Show when={a().kind === "navigation"}>
                <div class="browser-attention-target">
                    {t(a().popup ? "browser.attention.navigationPopupNote" : "browser.attention.navigationGoNote")}
                </div>
            </Show>
            <Show when={a().kind === "bypass"}>
                <div class="browser-attention-target">
                    <Rich k="browser.attention.bypass" params={{ sites: sitesOf(a()), time: minutesLabel(a().minutes ?? 60) }} />
                </div>
            </Show>
            <Show when={a().kind === "bypass" && asksEachLine(a().powers)}>
                <div class="browser-attention-target">
                    <Rich k="browser.attention.bypassAskEach" params={{ sites: sitesOf(a()) }} />
                </div>
            </Show>
            <Show when={a().kind === "bypass" && hasTerminalPower(a().powers) && !asksEachLine(a().powers)}>
                <div class="browser-attention-target">
                    <Rich k="browser.attention.bypassTerminal" params={{ sites: sitesOf(a()), limit: LATE_PROMPT_LIMIT }} />
                </div>
            </Show>
            <Show when={a().kind === "bypass" && hasEnterSpacePower(a().powers)}>
                <div class="browser-attention-target">
                    <Rich k="browser.attention.bypassEnterSpace" params={{ sites: sitesOf(a()) }} />
                </div>
            </Show>
            <Show when={a().kind === "bypass" && hasUploadsPower(a().powers)}>
                <div class="browser-attention-target">
                    <Rich k="browser.attention.bypassUploads" params={{ sites: sitesOf(a()) }} />
                </div>
            </Show>
            <Show when={a().window}>
                <div class="browser-attention-target">{t("browser.attention.popupWindow", { window: a().window ?? "" })}</div>
            </Show>
            <Show when={a().kind === "approval" && (a().fields?.length ?? 0) > 0}>
                <table class="browser-attention-fields">
                    <tbody>
                        <For each={a().fields}>
                            {(f) => (
                                <tr classList={{ "browser-attention-field-kept": f[2] === false }}>
                                    <td>{f[0]}</td>
                                    <td>
                                        {f[1]}
                                        {f[2] === false ? <span class="browser-attention-field-note"> {t("browser.attention.alreadyOnPage")}</span> : null}
                                    </td>
                                </tr>
                            )}
                        </For>
                    </tbody>
                </table>
            </Show>
            <Show when={a().kind === "approval" && a().action}>
                <div class="browser-attention-target">{t("browser.attention.sendsTo", { action: a().action ?? "" })}</div>
            </Show>
        </>
    );
}

/** The terminal line to run, and what the terminal shows around it. Both are the page's: text only, in boxes of their own. */
function LineDetails(props: Part & LineView): JSX.Element {
    const a = props.attention;
    const line = () => a().line ?? "";
    const shown = () => (a().long && !props.open() ? `${line().slice(0, LINE_PREVIEW_CHARS)}…` : line());
    return (
        <>
            <pre class="browser-attention-line" aria-label={t("browser.attention.lineLabel")}>
                {shown()}
            </pre>
            <Show when={a().long && !props.open()}>
                <div class="browser-attention-actions">
                    <Button density="compact" onClick={() => props.setOpen(true)}>
                        {t("browser.attention.showAll", { length: line().length })}
                    </Button>
                </div>
            </Show>
            <Show when={(a().rows?.length ?? 0) > 0}>
                <div class="browser-attention-target">{t("browser.attention.terminalShows")}</div>
                <pre class="browser-attention-page-text">{(a().rows ?? []).join("\n")}</pre>
            </Show>
        </>
    );
}

/** The buttons: the answer for the kind, Always (a bypass kept in Settings), and the way out. */
function AttentionActions(props: Part & LineView & { model: BrowserViewModel }): JSX.Element {
    const model = props.model;
    const a = props.attention;
    return (
        <div class="browser-attention-actions">
            <Show when={attentionLook(a()).back}>
                <Button
                    density={attentionLook(a()).density}
                    disabled={!model.canGoBackAtom()}
                    onClick={() => model.resolveAttention("back").catch(() => {})}
                >
                    {t("browser.attention.back")}
                </Button>
            </Show>
            <Show when={attentionLook(a()).yes}>
                {(yes) => (
                    <Button
                        tone={attentionLook(a()).tone}
                        density={attentionLook(a()).density}
                        disabled={a().kind === "line" && !!a().long && !props.open()}
                        title={a().kind === "line" && a().long && !props.open() ? t("browser.attention.showAllFirst") : undefined}
                        onClick={() => model.resolveAttention(attentionLook(a()).answer).catch(() => {})}
                    >
                        {yes()}
                    </Button>
                )}
            </Show>
            <Show when={a().kind === "bypass" && !a().local && hasTerminalPower(a().powers) && !asksEachLine(a().powers)}>
                <Button
                    density={attentionLook(a()).density}
                    title={t("browser.attention.allowAskTitle")}
                    onClick={() => model.resolveAttention("approve_ask").catch(() => {})}
                >
                    {t("browser.attention.allowAsk")}
                </Button>
            </Show>
            <Show when={a().kind === "bypass" && !a().local}>
                <Button
                    density={attentionLook(a()).density}
                    title={t("browser.attention.alwaysTitle")}
                    onClick={() => model.resolveAttention("approve_always").catch(() => {})}
                >
                    {t("browser.attention.always", {
                        agent: a().agent ? agentDisplayName(a().agent ?? "") : t("browser.attention.thisAgent"),
                    })}
                </Button>
            </Show>
            <Button
                density={attentionLook(a()).density}
                onClick={() => model.resolveAttention("cancel").catch(() => {})}
            >
                {attentionLook(a()).no}
            </Button>
        </div>
    );
}

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The request a browser pane puts over its page when an agent or the page waits on the person: its words for each kind, and its buttons. */

import { createSignal, For, Match, Show, Switch, type Accessor, type JSX } from "solid-js";
import { Button } from "@/app/element/ui";
import { tip } from "@/app/element/tip";
import { attentionLook, dialogMessage } from "./attention-kinds";
import { asksEachLine, hasTerminalPower, LATE_PROMPT_LIMIT, LINE_PREVIEW_CHARS, minutesLabel, siteLabel } from "./bypass";
import type { BrowserViewModel } from "./browser-model";

type Part = { attention: Accessor<BrowserAttention> };
/** A long terminal line is read in full ("Show all") before Run it works. */
type LineView = { open: Accessor<boolean>; setOpen: (open: boolean) => void };

export function BrowserAttentionBanner(props: { model: BrowserViewModel; attention: Accessor<BrowserAttention> }): JSX.Element {
    const a = props.attention;
    const [open, setOpen] = createSignal(false);
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

/** The icon and the sentence that says who wants what. */
function AttentionHead(props: Part): JSX.Element {
    const a = props.attention;
    return (
        <div class="browser-attention-head">
            <i class={`fa-solid ${attentionLook(a()).icon}`} aria-hidden="true" />
            <Switch
                fallback={
                    <span>
                        <b>{a().agent || "An agent"}</b>{" "}
                        {a().kind === "handoff"
                            ? "needs you: "
                            : a().kind === "bypass"
                              ? "asks you to auto-approve its clicks: "
                              : "wants to: "}
                        {a().kind === "handoff" || a().kind === "bypass" ? a().reason : a().what}
                    </span>
                }
            >
                <Match when={attentionLook(a()).text}>{(text) => <span>{text()}</span>}</Match>
                <Match when={a().kind === "line"}>
                    <span>
                        <b>{a().agent || "An agent"}</b> typed a line in the terminal on <b>{siteLabel(a().origin ?? "")}</b> and wants to run it
                        {a().reason ? <>: {a().reason}</> : null}
                    </span>
                </Match>
                <Match when={a().kind === "take"}>
                    <span>
                        <b>{a().agent || "An agent"}</b> wants to drive this pane
                        {a().reason ? <> ({a().reason})</> : null}
                    </span>
                </Match>
                <Match when={a().kind === "navigation"}>
                    <span>
                        This pane is limited to the sites it was opened for. The page wants to{" "}
                        {a().popup ? "open a window at " : "go to "}
                        <b>{a().origin}</b>.
                    </span>
                </Match>
                <Match when={a().kind === "dialog"}>
                    <span>
                        {a().dialogKind === "beforeunload" ? (
                            <>
                                The page at <b>{a().origin || "this site"}</b> asks whether to leave it
                            </>
                        ) : (
                            <>
                                The page at <b>{a().origin || "this site"}</b> asks
                            </>
                        )}
                        {a().agent ? (
                            <>
                                {" "}
                                (after <b>{a().agent}</b>'s action)
                            </>
                        ) : null}
                        :
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
                <div class="browser-attention-target">Asked because {a().why}.</div>
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
                    {a().popup
                        ? "Allow adds this site to the pane's list; then click again to open the window."
                        : "Allow adds this site to the pane's list and goes there. A form's data isn't sent again."}
                </div>
            </Show>
            <Show when={a().kind === "bypass"}>
                <div class="browser-attention-target">
                    On <b>{(a().origins ?? []).map(siteLabel).join(", ")}</b>, for{" "}
                    <b>{minutesLabel(a().minutes ?? 60)}</b>: its clicks that submit, delete or send go
                    through without asking you, and so does the page's own "Are you sure?" that follows one. It still can't type passwords or codes or run scripts, and
                    payments still ask. Stop it any time from the red strip.
                </div>
            </Show>
            <Show when={a().kind === "bypass" && asksEachLine(a().powers)}>
                <div class="browser-attention-target">
                    It also asks to <b>type in the terminal</b> on {(a().origins ?? []).map(siteLabel).join(", ")}: you are shown each line
                    here, with Run it and Don't run, before it goes in. Passwords and codes stay yours to type.
                </div>
            </Show>
            <Show when={a().kind === "bypass" && hasTerminalPower(a().powers) && !asksEachLine(a().powers)}>
                <div class="browser-attention-target">
                    It also asks to <b>run commands in the terminal</b> on {(a().origins ?? []).map(siteLabel).join(", ")}:
                    it types real keys and presses Enter with no question per line. You can read every line in the
                    pane's Log, and Stop or Pause end it at once. Passwords and codes stay yours to type.{" "}
                    One limit: {LATE_PROMPT_LIMIT}
                </div>
            </Show>
            <Show when={a().window}>
                <div class="browser-attention-target">In its popup window: {a().window}</div>
            </Show>
            <Show when={a().kind === "approval" && (a().fields?.length ?? 0) > 0}>
                <table class="browser-attention-fields">
                    <tbody>
                        <For each={a().fields}>
                            {(f) => (
                                <tr>
                                    <td>{f[0]}</td>
                                    <td>{f[1]}</td>
                                </tr>
                            )}
                        </For>
                    </tbody>
                </table>
            </Show>
            <Show when={a().kind === "approval" && a().action}>
                <div class="browser-attention-target">Sends to {a().action}</div>
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
            <pre class="browser-attention-line" aria-label="The line that will run">
                {shown()}
            </pre>
            <Show when={a().long && !props.open()}>
                <div class="browser-attention-actions">
                    <Button density="compact" onClick={() => props.setOpen(true)}>
                        Show all ({line().length} characters)
                    </Button>
                </div>
            </Show>
            <Show when={(a().rows?.length ?? 0) > 0}>
                <div class="browser-attention-target">What the terminal shows now (the page's words, not AgentMux's):</div>
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
                    Back
                </Button>
            </Show>
            <Show when={attentionLook(a()).yes}>
                {(yes) => (
                    <Button
                        tone={attentionLook(a()).tone}
                        density={attentionLook(a()).density}
                        disabled={a().kind === "line" && !!a().long && !props.open()}
                        title={a().kind === "line" && a().long && !props.open() ? "Show all of the line first" : undefined}
                        onClick={() => model.resolveAttention(attentionLook(a()).answer).catch(() => {})}
                    >
                        {yes()}
                    </Button>
                )}
            </Show>
            <Show when={a().kind === "bypass" && !a().local && hasTerminalPower(a().powers) && !asksEachLine(a().powers)}>
                <Button
                    density={attentionLook(a()).density}
                    title="Allow it for the time you chose, but see each terminal line here and press Run it before it goes in"
                    onClick={() => model.resolveAttention("approve_ask").catch(() => {})}
                >
                    Allow, but show me each line
                </Button>
            </Show>
            <Show when={a().kind === "bypass" && !a().local}>
                <Button
                    density={attentionLook(a()).density}
                    title="Keep this: it is added to Settings, Agents on sites, where you can revoke it"
                    onClick={() => model.resolveAttention("approve_always").catch(() => {})}
                >
                    Always for {a().agent ?? "this agent"} on this site
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

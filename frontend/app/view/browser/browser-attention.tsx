// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The request a browser pane puts over its page when an agent or the page waits on the person: its words for each kind, and its buttons. */

import { For, Match, Show, Switch, type Accessor, type JSX } from "solid-js";
import { Button } from "@/app/element/ui";
import { tip } from "@/app/element/tip";
import { attentionLook, dialogMessage } from "./attention-kinds";
import { hasTerminalPower, LATE_PROMPT_LIMIT, minutesLabel, siteLabel } from "./bypass";
import type { BrowserViewModel } from "./browser-model";

type Part = { attention: Accessor<BrowserAttention> };

export function BrowserAttentionBanner(props: { model: BrowserViewModel; attention: Accessor<BrowserAttention> }): JSX.Element {
    const a = props.attention;
    return (
        <div
            class="browser-attention"
            classList={{ "browser-attention--agent": attentionLook(a()).agent }}
            data-kind={a().kind}
            role="alertdialog"
            aria-live="assertive"
        >
            <AttentionHead attention={a} />
            <AttentionDetails attention={a} />
            <AttentionActions model={props.model} attention={a} />
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
function AttentionDetails(props: Part): JSX.Element {
    const a = props.attention;
    return (
        <>
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
            <Show when={a().kind === "bypass" && hasTerminalPower(a().powers)}>
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

/** The buttons: the answer for the kind, Always (a bypass kept in Settings), and the way out. */
function AttentionActions(props: Part & { model: BrowserViewModel }): JSX.Element {
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
                        onClick={() => model.resolveAttention(attentionLook(a()).answer).catch(() => {})}
                    >
                        {yes()}
                    </Button>
                )}
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

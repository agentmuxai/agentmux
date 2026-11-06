// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Messages panel: the incoming jekts of one agent pane, in a column beside the conversation, or a narrow rail
 * with a count when it is closed (docs/specs/SPEC_JEKT_SIDE_PANEL_RESEARCH_AND_DESIGN_2026_11_04.md §5). Each
 * message is read from the pane's own document nodes and drawn with the same tags and metadata as its row in the
 * conversation. Messages that wait on the person sit above the list and never clear by being scrolled past.
 */

import {
    createEffect,
    createMemo,
    createSignal,
    For,
    on,
    onCleanup,
    Show,
    untrack,
    type Accessor,
    type JSX,
} from "solid-js";

import type { DocumentNode, JektMessageNode } from "@/app/agent-model/node-types";
import { Button } from "@/app/element/ui/Button";
import { Select } from "@/app/element/ui/inputs";
import { Pill } from "@/app/element/ui/Pill";
import { SidePanel } from "@/app/element/ui/SidePanel";
import { AttachmentStrip } from "../attachments/AttachmentStrip";
import { JektMeta, JektTags } from "../components/JektParts";
import { PreviewLines } from "../components/PreviewLines";
import { awaitingYou, dismissAwaiting } from "../hooks/useJektsAwaitingYou";
import { proseDoc, rawDoc } from "../preview-text/docs";
import { scrollMessagesTo } from "../scroll/one-way-flow";
import { createNarrow, publishHeaderButton } from "./messages-overlay";
import "./messages-panel.scss";
import {
    applyFilter,
    badgeText,
    isUnread,
    panelMessages,
    topSenders,
    unreadCount,
    type MessagesFilter,
} from "./messages-select";
import type { MessagesPanelControls } from "./messages-state";

/** How long the panel must be open on a focused window before its messages count as seen. */
export const SEEN_AFTER_MS = 1500;

/** Within this distance of the end, the list counts as at its tail and follows new messages. */
const TAIL_SLACK_PX = 24;

const timeOf = (ms: number): string => new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });

const firstLine = (text: string): string => text.split("\n").find((l) => l.trim()) ?? "";

export interface MessagesPanelProps {
    blockId: string;
    controls: MessagesPanelControls;
    nodes: Accessor<readonly DocumentNode[]>;
    /** Scroll the conversation to a node. */
    jumpTo: (nodeId: string) => void;
    /** The pane's zoom: the panel scales with the conversation. */
    zoom: number;
}

/** The unread count, the waiting set and the list the panel and its rail both draw from. */
function createMessagesModel(props: MessagesPanelProps) {
    const messages = createMemo(() => panelMessages(props.nodes()));
    const waiting = createMemo(() => awaitingYou(props.blockId, props.nodes()));
    const waitingIds = createMemo(() => new Set(waiting().map((m) => m.id)));
    const unread = createMemo(() => unreadCount(messages(), props.controls.state().seen));
    return { messages, waiting, waitingIds, unread };
}

export function MessagesPanel(props: MessagesPanelProps): JSX.Element {
    // The model is made only while the panel is on: a pane that has not turned it on does none of its work.
    return (
        <Show when={props.controls.state().enabled}>
            <MessagesDock {...props} />
        </Show>
    );
}

function MessagesDock(props: MessagesPanelProps): JSX.Element {
    const model = createMessagesModel(props);
    const { controls } = props;
    let dock: HTMLDivElement | undefined;
    const narrow = createNarrow(
        () => dock?.parentElement,
        () => props.zoom
    );
    publishHeaderButton(
        props.blockId,
        narrow,
        model.unread,
        () => model.waiting().length > 0,
        () => controls.setOverlayOpen(!controls.overlayOpen())
    );
    // Wide: the column or its rail beside the conversation. Narrow: the column over it, only while it is opened.
    return (
        <div
            ref={dock}
            class="messages-dock"
            classList={{
                "messages-dock--overlay": narrow(),
                "messages-dock--idle": narrow() && !controls.overlayOpen(),
            }}
            style={{ zoom: props.zoom }}
        >
            <Show
                when={narrow()}
                fallback={
                    <Show
                        when={controls.state().open}
                        fallback={<MessagesRail model={model} onOpen={() => controls.patch({ open: true })} />}
                    >
                        <MessagesColumn {...props} model={model} onClose={() => controls.patch({ open: false })} />
                    </Show>
                }
            >
                <Show when={controls.overlayOpen()}>
                    <MessagesColumn {...props} model={model} onClose={() => controls.setOverlayOpen(false)} />
                </Show>
            </Show>
        </div>
    );
}

type Model = ReturnType<typeof createMessagesModel>;

/** The closed panel: a thin rail on the right edge with the count and a dot while something needs the person. */
function MessagesRail(props: { model: Model; onOpen: () => void }): JSX.Element {
    const label = () => {
        const n = props.model.unread();
        const needs = props.model.waiting().length;
        return `Messages${n ? `, ${n} unread` : ""}${needs ? `, ${needs} need you` : ""}`;
    };
    return (
        <aside class="messages-rail" aria-label="Messages">
            <Button
                variant="chrome"
                class="messages-rail__button"
                onClick={props.onOpen}
                aria-label={label()}
                title={label()}
            >
                <span class="messages-rail__text">Messages</span>
                <Show when={props.model.unread() > 0}>
                    <Pill tone="accent">{badgeText(props.model.unread())}</Pill>
                </Show>
                <Show when={props.model.waiting().length > 0}>
                    <span class="messages-rail__needs" role="img" aria-label="Needs you" />
                </Show>
            </Button>
        </aside>
    );
}

/**
 * The panel counts as read once it has been open on a focused window for a moment. Messages that need the person
 * are tracked apart (the waiting set) and stay lit whatever is seen. Returns "Mark all read".
 */
function markSeenWhenRead(model: Model, controls: MessagesPanelControls): () => void {
    createEffect(() => {
        if (model.unread() === 0) return;
        let timer: ReturnType<typeof setTimeout> | undefined;
        // An unfocused window waits for its focus, then counts the moment again.
        const arm = () => {
            timer = setTimeout(() => {
                if (document.hasFocus()) controls.patch({ seen: Date.now() });
                else window.addEventListener("focus", arm, { once: true });
            }, SEEN_AFTER_MS);
        };
        arm();
        onCleanup(() => {
            clearTimeout(timer);
            window.removeEventListener("focus", arm);
        });
    });
    return () => controls.patch({ seen: Date.now() });
}

/**
 * How the list moves: it follows the tail while the reader is at it, counts what arrived below while the reader is
 * up the list (arrivals: the pane's messages, not what a filter shows), and scrolls to a message the conversation
 * asked for. Every move goes through scrollMessagesTo.
 */
function createListBehaviour(
    getList: () => HTMLElement | undefined,
    count: Accessor<number>,
    controls: MessagesPanelControls,
    onRequested: (nodeId: string) => void
) {
    const [newBelow, setNewBelow] = createSignal(0);
    let atTail = true;
    createEffect(
        on(count, (now, before) => {
            const list = getList();
            if (!list || before === undefined || now <= before) return;
            if (atTail) queueMicrotask(() => scrollMessagesTo(list, list.scrollHeight));
            else setNewBelow((n) => n + (now - before));
        })
    );
    // A click on a message's stub in the conversation: show it here, open.
    createEffect(
        on(
            () => controls.request(),
            (req) => {
                if (!req) return;
                // Handled once: a column that mounts again (closed to the rail, then reopened) must not replay it.
                controls.clearRequest();
                onRequested(req.nodeId);
                queueMicrotask(() => {
                    const list = getList();
                    const row = list?.querySelector<HTMLElement>(`[data-msg-id="${CSS.escape(req.nodeId)}"]`);
                    if (list && row) scrollMessagesTo(list, row.offsetTop, "smooth");
                });
            }
        )
    );
    const onScroll = () => {
        const list = getList();
        if (!list) return;
        atTail = list.scrollHeight - list.scrollTop - list.clientHeight <= TAIL_SLACK_PX;
        if (atTail) setNewBelow(0);
    };
    return {
        newBelow,
        onScroll,
        toTail: () => {
            const list = getList();
            if (list) scrollMessagesTo(list, list.scrollHeight, "smooth");
        },
    };
}

function MessagesColumn(props: MessagesPanelProps & { model: Model; onClose: () => void }): JSX.Element {
    const { model, controls } = props;
    const [filter, setFilter] = createSignal<MessagesFilter>("all");
    const [opened, setOpened] = createSignal<ReadonlySet<string>>(new Set());
    let list: HTMLElement | undefined;

    const visible = createMemo(() =>
        applyFilter(model.messages(), filter(), { seen: controls.state().seen, waiting: model.waitingIds() })
    );
    const filterOptions = createMemo(() => [
        { value: "all", label: "All" },
        { value: "needs-me", label: "Needs me" },
        { value: "unread", label: "Unread" },
        ...topSenders(model.messages()).map((from) => ({ value: `from:${from}`, label: from })),
    ]);

    const toggle = (id: string) =>
        setOpened((prev) => {
            const next = new Set(prev);
            if (!next.delete(id)) next.add(id);
            return next;
        });

    const markAllRead = markSeenWhenRead(model, controls);
    const moves = createListBehaviour(
        () => list,
        () => model.messages().length,
        controls,
        (nodeId) => {
            setFilter("all");
            setOpened((prev) => new Set(prev).add(nodeId));
        }
    );

    // A filter changes what the list holds and where it sits, and no scroll event says so: look again.
    createEffect(on(filter, () => queueMicrotask(moves.onScroll), { defer: true }));

    return (
        <SidePanel
            docked
            class="messages-panel"
            label="Messages"
            title={
                <>
                    Messages
                    <Show when={model.unread() > 0}>
                        {" "}
                        <Pill tone="accent">{badgeText(model.unread())}</Pill>
                    </Show>
                </>
            }
            onClose={props.onClose}
        >
            <Show when={model.waiting().length > 0}>
                <NeedsYou
                    waiting={model.waiting()}
                    onShow={props.jumpTo}
                    onDismiss={() => dismissAwaiting(props.blockId, untrack(model.waiting))}
                />
            </Show>
            <div class="messages-panel__tools">
                <Select
                    density="compact"
                    ariaLabel="Show"
                    value={filter()}
                    options={filterOptions()}
                    onChange={(v) => setFilter(v as MessagesFilter)}
                />
                <Button variant="link" disabled={model.unread() === 0} onClick={markAllRead}>
                    Mark all read
                </Button>
            </div>
            <ul class="messages-panel__list" ref={(el) => (list = el)} onScroll={moves.onScroll}>
                <For each={visible()} fallback={<li class="messages-panel__empty">No messages</li>}>
                    {(m) => (
                        <MessageRow
                            node={m}
                            open={opened().has(m.id)}
                            unread={isUnread(m, controls.state().seen)}
                            needsYou={model.waitingIds().has(m.id)}
                            onToggle={() => toggle(m.id)}
                            onShow={() => props.jumpTo(m.id)}
                        />
                    )}
                </For>
            </ul>
            <Show when={moves.newBelow() > 0}>
                <Button class="messages-panel__new" tone="accent" icon="arrow-down" onClick={moves.toTail}>
                    {moves.newBelow()} new
                </Button>
            </Show>
        </SidePanel>
    );
}

/** Above the list: what waits on the person, with the same Dismiss as the card above the composer. */
function NeedsYou(props: {
    waiting: readonly JektMessageNode[];
    onShow: (nodeId: string) => void;
    onDismiss: () => void;
}): JSX.Element {
    const newest = () => props.waiting[props.waiting.length - 1];
    return (
        <section class="messages-panel__needs" aria-label="Needs you">
            <strong>Needs you</strong>
            <span>
                {newest().from} sent a sensitive message. Confirm it in this conversation; another agent's reply does
                not count.
                <Show when={props.waiting.length > 1}> {props.waiting.length - 1} earlier ones wait too.</Show>
            </span>
            <div class="messages-panel__needs-actions">
                <Button variant="link" onClick={() => props.onShow(newest().id)}>
                    Show message
                </Button>
                <Button variant="link" onClick={props.onDismiss}>
                    Dismiss
                </Button>
            </div>
        </section>
    );
}

function MessageRow(props: {
    node: JektMessageNode;
    open: boolean;
    unread: boolean;
    needsYou: boolean;
    onToggle: () => void;
    onShow: () => void;
}): JSX.Element {
    const bodyDoc = createMemo(() => proseDoc(props.node.message));
    const rawPayload = createMemo(() => rawDoc(props.node.raw));
    return (
        <li
            class="messages-row"
            classList={{ "messages-row--unread": props.unread, "messages-row--needs": props.needsYou }}
            data-msg-id={props.node.id}
        >
            <Button variant="row" class="messages-row__head" pressed={props.open} onClick={props.onToggle}>
                <span class="messages-row__from">{props.node.from}</span>
                <Show when={props.unread}>
                    <span class="messages-row__dot" role="img" aria-label="Unread" />
                </Show>
                <time class="messages-row__time">{timeOf(props.node.timestamp)}</time>
            </Button>
            <Show
                when={props.open}
                fallback={
                    <>
                        <div class="messages-row__peek">{firstLine(props.node.message)}</div>
                        <div class="messages-row__tags">
                            <JektTags node={props.node} />
                        </div>
                    </>
                }
            >
                <div class="messages-row__tags">
                    <JektTags node={props.node} />
                </div>
                <Show when={props.node.attachments?.length}>
                    <AttachmentStrip attachments={props.node.attachments!} />
                </Show>
                <div class="messages-row__body">
                    <PreviewLines doc={bodyDoc()} linkify />
                </div>
                <JektMeta node={props.node} />
                <details class="messages-row__raw">
                    <summary>Raw payload</summary>
                    <div class="messages-row__raw-body">
                        <PreviewLines doc={rawPayload()} />
                    </div>
                </details>
                <div class="messages-row__actions">
                    <Button variant="link" onClick={props.onShow}>
                        Show in conversation
                    </Button>
                </div>
            </Show>
        </li>
    );
}

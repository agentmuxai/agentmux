// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Remotes pane (docs/specs/SPEC_REMOTES_PANE_2026_10_05.md §4.2–§4.3):
// every remote machine in sections, a row per remote, and an inline detail
// panel with its actions and settings.

import { createMemo, createSignal, For, Show, type JSX } from "solid-js";
import { ContextMenu, type ContextMenuItem } from "@/app/components/context-menu";
import type { RemoteRecord } from "@/app/store/rpc-api/remotes";
import type { RemotesViewModel } from "./remotes-model";
import {
    displayName,
    groupRemotes,
    helperLabel,
    isHidden,
    isPinned,
    platformLabel,
    plural,
    REMOTE_COLORS,
    remoteColor,
    statusLabel,
    type RemoteGroup,
} from "./remotes-sections";
import "./remotes-view.scss";

interface MenuState {
    items: ContextMenuItem[];
    x: number;
    y: number;
}

export function RemotesView(props: { model: RemotesViewModel }): JSX.Element {
    const model = props.model;
    const groups = createMemo(() => groupRemotes(model.records(), model.filter()));
    const [hiddenOpen, setHiddenOpen] = createSignal(false);
    const [menu, setMenu] = createSignal<MenuState | null>(null);

    const showMenu = (r: RemoteRecord, e: MouseEvent) => {
        e.preventDefault();
        e.stopPropagation();
        setMenu({ items: rowMenu(model, r), x: e.clientX, y: e.clientY });
    };

    return (
        <div class="remotes-view">
            <div class="remotes-toolbar">
                <input
                    class="remotes-filter"
                    type="search"
                    placeholder="Filter remotes"
                    aria-label="Filter remotes"
                    value={model.filter()}
                    onInput={(e) => model.setFilter(e.currentTarget.value)}
                />
                <button class="remotes-icon-button" title="Refresh" aria-label="Refresh" onClick={() => void model.refresh()}>
                    <i class="fa fa-rotate-right" />
                </button>
            </div>
            <Show when={model.error()}>
                <div class="remotes-error" role="alert">
                    {model.error()}
                </div>
            </Show>
            <Show when={model.notice()}>
                <div class="remotes-notice" role="status">
                    <span>{model.notice()}</span>
                    <button class="remotes-link" onClick={() => model.setNotice("")}>
                        Dismiss
                    </button>
                </div>
            </Show>
            <div class="remotes-list">
                <Show when={model.loaded()} fallback={<div class="remotes-empty">Loading…</div>}>
                    <Show when={groups().length > 0} fallback={<EmptyList filter={model.filter()} />}>
                        <For each={groups()}>
                            {(group) => (
                                <Section
                                    group={group}
                                    model={model}
                                    open={group.section !== "hidden" || hiddenOpen() || !!model.filter().trim()}
                                    onToggle={group.section === "hidden" ? () => setHiddenOpen(!hiddenOpen()) : undefined}
                                    onMenu={showMenu}
                                />
                            )}
                        </For>
                    </Show>
                </Show>
            </div>
            <Show when={menu()}>
                {(m) => <ContextMenu items={m().items} x={m().x} y={m().y} onClose={() => setMenu(null)} />}
            </Show>
        </div>
    );
}

function EmptyList(props: { filter: string }): JSX.Element {
    return (
        <div class="remotes-empty">
            <Show
                when={props.filter.trim()}
                fallback={
                    <>
                        No remotes yet. Hosts in <code>~/.ssh/config</code> appear here, and so does any host you connect
                        to from a terminal's connection menu.
                    </>
                }
            >
                No remotes match “{props.filter.trim()}”.
            </Show>
        </div>
    );
}

function Section(props: {
    group: RemoteGroup;
    model: RemotesViewModel;
    open: boolean;
    onToggle?: () => void;
    onMenu: (r: RemoteRecord, e: MouseEvent) => void;
}): JSX.Element {
    return (
        <section class="remotes-section" data-section={props.group.section}>
            <Show
                when={props.onToggle}
                fallback={<h3 class="remotes-section-heading">{props.group.label}</h3>}
            >
                <h3 class="remotes-section-heading">
                    <button class="remotes-section-toggle" aria-expanded={props.open} onClick={() => props.onToggle?.()}>
                        <i class={`fa fa-chevron-${props.open ? "down" : "right"}`} />
                        {props.group.label} ({props.group.records.length})
                    </button>
                </h3>
            </Show>
            <Show when={props.open}>
                <For each={props.group.records}>
                    {(r) => <Row record={r} model={props.model} onMenu={props.onMenu} />}
                </For>
            </Show>
        </section>
    );
}

function Row(props: {
    record: RemoteRecord;
    model: RemotesViewModel;
    onMenu: (r: RemoteRecord, e: MouseEvent) => void;
}): JSX.Element {
    const model = props.model;
    const r = () => props.record;
    const expanded = () => model.expanded() === r().name;
    const nick = () => displayName(r()) !== r().name;
    const activity = () => {
        const parts: string[] = [];
        if (r().sessions != null && r().sessions! > 0) parts.push(plural(r().sessions!, "session"));
        if (r().agents.length > 0) parts.push(plural(r().agents.length, "agent"));
        return parts.join(" · ");
    };
    const stop = (e: MouseEvent) => e.stopPropagation();

    return (
        <div class="remotes-item" classList={{ "is-expanded": expanded() }}>
            <div
                class="remotes-row"
                role="button"
                tabIndex={0}
                aria-expanded={expanded()}
                data-remote={r().name}
                onClick={() => model.toggleExpanded(r().name)}
                onDblClick={() => void model.run("New terminal", () => model.newTerminal(r().name))}
                onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === " ") {
                        e.preventDefault();
                        model.toggleExpanded(r().name);
                    }
                }}
                onContextMenu={(e) => props.onMenu(r(), e)}
            >
                <span
                    class="remotes-swatch"
                    classList={{ "is-empty": !remoteColor(r()) }}
                    style={remoteColor(r()) ? { background: remoteColor(r()) } : undefined}
                />
                <span class={`remotes-dot is-${r().status.state || "disconnected"}`} title={statusLabel(r().status)} />
                <span class="remotes-name">
                    {displayName(r())}
                    <Show when={nick()}>
                        <span class="remotes-realname">{r().name}</span>
                    </Show>
                </span>
                <Show when={platformLabel(r().platform)}>
                    <span class="remotes-tag">{platformLabel(r().platform)}</span>
                </Show>
                <span class="remotes-meta">
                    <Show when={helperLabel(r().helper)}>
                        <span class={`remotes-helper is-${r().helper.state}`}>{helperLabel(r().helper)}</span>
                    </Show>
                    <Show when={activity()}>
                        <span class="remotes-activity">{activity()}</span>
                    </Show>
                </span>
                <span class="remotes-row-actions" onClick={stop} onDblClick={stop}>
                    <button
                        class="remotes-icon-button"
                        title="New terminal"
                        aria-label={`New terminal on ${r().name}`}
                        onClick={() => void model.run("New terminal", () => model.newTerminal(r().name))}
                    >
                        <i class="fa fa-terminal" />
                    </button>
                    <button
                        class="remotes-icon-button"
                        title="Browse files"
                        aria-label={`Browse files on ${r().name}`}
                        onClick={() => void model.run("Browse files", () => model.browseFiles(r().name))}
                    >
                        <i class="fa fa-folder-open" />
                    </button>
                </span>
            </div>
            <Show when={expanded()}>
                <Detail record={r()} model={model} />
            </Show>
        </div>
    );
}

function Detail(props: { record: RemoteRecord; model: RemotesViewModel }): JSX.Element {
    const model = props.model;
    const r = () => props.record;
    const connected = () => r().status.state === "connected" || r().status.state === "connecting";
    const durable = () => r().settings?.["term:durable"];
    const run = (what: string, action: () => Promise<void>) => () => void model.run(what, action);

    const commitNickname = (value: string) => {
        const current = (r().settings?.["display:name"] as string | undefined) ?? "";
        if (value.trim() === current.trim()) return;
        void model.run("Rename", () => model.setNickname(r().name, value));
    };

    return (
        <div class="remotes-detail">
            <Show when={r().status.state === "error" && r().status.error}>
                <div class="remotes-detail-error">{r().status.error}</div>
            </Show>
            <div class="remotes-detail-actions">
                <Show
                    when={connected()}
                    fallback={<button onClick={run("Connect", () => model.connect(r().name))}>Connect</button>}
                >
                    <button onClick={run("Disconnect", () => model.disconnect(r().name))}>Disconnect</button>
                </Show>
                <Show when={r().kind === "ssh"}>
                    <button onClick={run("Sessions", () => model.showSessions(r().name))}>Sessions…</button>
                </Show>
                <button onClick={run(isPinned(r()) ? "Unpin" : "Pin", () => model.setPinned(r().name, !isPinned(r())))}>
                    {isPinned(r()) ? "Unpin" : "Pin"}
                </button>
                <button onClick={run(isHidden(r()) ? "Unhide" : "Hide", () => model.setHidden(r().name, !isHidden(r())))}>
                    {isHidden(r()) ? "Unhide" : "Hide"}
                </button>
                <Show when={r().sources.includes("recent")}>
                    <button onClick={run("Forget", () => model.forget(r().name))}>Forget</button>
                </Show>
            </div>
            <div class="remotes-settings">
                <label class="remotes-setting">
                    <span>Nickname</span>
                    <input
                        type="text"
                        placeholder={r().name}
                        value={(r().settings?.["display:name"] as string | undefined) ?? ""}
                        onBlur={(e) => commitNickname(e.currentTarget.value)}
                        onKeyDown={(e) => {
                            if (e.key === "Enter") commitNickname(e.currentTarget.value);
                        }}
                    />
                </label>
                <div class="remotes-setting">
                    <span>Colour</span>
                    <div class="remotes-colors" role="radiogroup" aria-label="Colour">
                        <button
                            class="remotes-color is-none"
                            role="radio"
                            aria-checked={!remoteColor(r())}
                            title="None"
                            aria-label="None"
                            onClick={run("Colour", () => model.setColor(r().name, null))}
                        />
                        <For each={REMOTE_COLORS}>
                            {(c) => (
                                <button
                                    class="remotes-color"
                                    role="radio"
                                    aria-checked={remoteColor(r())?.toLowerCase() === c.value}
                                    title={c.name}
                                    aria-label={c.name}
                                    style={{ background: c.value }}
                                    onClick={run("Colour", () => model.setColor(r().name, c.value))}
                                />
                            )}
                        </For>
                    </div>
                </div>
                <Show when={r().kind === "ssh"}>
                    <label class="remotes-setting">
                        <span>Keep sessions alive</span>
                        <select
                            value={durable() === true ? "on" : durable() === false ? "off" : "default"}
                            onChange={(e) => {
                                const v = e.currentTarget.value;
                                void model.run("Keep sessions alive", () =>
                                    model.setDurable(r().name, v === "on" ? true : v === "off" ? false : null)
                                );
                            }}
                        >
                            <option value="default">As in Settings</option>
                            <option value="on">On</option>
                            <option value="off">Off</option>
                        </select>
                    </label>
                </Show>
            </div>
            <Show when={r().agents.length > 0}>
                <div class="remotes-agents">
                    <span>Agents always allowed here:</span> {r().agents.join(", ")}
                </div>
            </Show>
        </div>
    );
}

/** The right-click menu: the row's actions (§4.2). */
export function rowMenu(model: RemotesViewModel, r: RemoteRecord): ContextMenuItem[] {
    const act = (label: string, action: () => Promise<void>, extra: Partial<ContextMenuItem> = {}): ContextMenuItem => ({
        type: "action",
        label,
        onSelect: () => void model.run(label, action),
        ...extra,
    });
    const connected = r.status.state === "connected" || r.status.state === "connecting";
    const items: ContextMenuItem[] = [
        act("New terminal", () => model.newTerminal(r.name)),
        act("Browse files", () => model.browseFiles(r.name)),
    ];
    if (r.kind === "ssh") items.push(act("Sessions…", () => model.showSessions(r.name)));
    items.push({ type: "separator" });
    items.push(
        connected ? act("Disconnect", () => model.disconnect(r.name)) : act("Connect", () => model.connect(r.name))
    );
    items.push({ type: "separator" });
    items.push(
        isPinned(r) ? act("Unpin", () => model.setPinned(r.name, false)) : act("Pin", () => model.setPinned(r.name, true)),
        isHidden(r) ? act("Unhide", () => model.setHidden(r.name, false)) : act("Hide", () => model.setHidden(r.name, true))
    );
    if (r.sources.includes("recent")) items.push(act("Forget", () => model.forget(r.name)));
    items.push({ type: "separator" });
    items.push({
        type: "action",
        label: "Settings",
        onSelect: () => model.setExpanded(r.name),
    });
    return items;
}

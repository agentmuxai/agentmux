// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Remotes section of Connectors (docs/specs/SPEC_REMOTES_PANE_2026_10_05.md
// §4.2–§4.3): every remote machine as a card, grouped, with its actions and
// settings opening below the card's row
// (docs/specs/SPEC_CONNECTORS_CARDS_2026_10_13.md §3.5).

import { createEffect, createMemo, createSignal, For, Show, type JSX } from "solid-js";
import { ContextMenu, type ContextMenuItem } from "@/app/components/context-menu";
import type { RemoteRecord } from "@/app/store/rpc-api/remotes";
import { MENU_ICON } from "@/app/util/menu-icons";
import type { RemotesViewModel } from "./remotes-model";
import {
    displayName,
    groupRemotes,
    helperLabel,
    isHidden,
    isPinned,
    platformLabel,
    plural,
    remoteColor,
    statusLabel,
    type RemoteGroup,
} from "./remotes-sections";
import "./remotes-view.scss";
import {
    Button,
    Card,
    CardGrid,
    EmptyState,
    FilterInput,
    IconButton,
    Notice,
    PaneBody,
    PaneScroll,
    SectionHeader,
    TextInput,
    type StatusTone,
} from "@/app/element/ui";
import { focusOnOpen } from "@/util/focusutil";
import { connectionName, parseDestination } from "./add-remote";
import { DetailActions, DetailAgents, DetailSettings, TestResult } from "./remotes-detail-parts";
import { connectionAction, runConnectionAction } from "./remotes-connection";
import { tip } from "@/app/element/tip";

interface MenuState {
    items: ContextMenuItem[];
    x: number;
    y: number;
}

/** The status dot's tone for a remote's connection state. */
const STATUS_TONES: Record<string, StatusTone> = {
    connected: "ok",
    connecting: "busy",
    error: "error",
    disconnected: "idle",
};

export function RemotesView(props: { model: RemotesViewModel }): JSX.Element {
    const model = props.model;
    const groups = createMemo(() => groupRemotes(model.records(), model.filter()));
    // Sections are keyed by id and the grids by name, not by object: a refresh
    // brings new records and groups, and keying on those would rebuild every
    // card and the open details (losing focus and a half-typed nickname).
    const byName = createMemo(() => new Map(model.records().map((r) => [r.name, r])));
    const [hiddenOpen, setHiddenOpen] = createSignal(false);
    const [menu, setMenu] = createSignal<MenuState | null>(null);
    const [adding, setAdding] = createSignal(false);
    // Another pane's link named a host: open its card.
    createEffect(() => {
        const want = model.expandRequest();
        if (want) model.takeExpandRequest(want);
    });

    const showMenu = (r: RemoteRecord, e: MouseEvent) => {
        e.preventDefault();
        e.stopPropagation();
        setMenu({ items: rowMenu(model, r), x: e.clientX, y: e.clientY });
    };

    return (
        <PaneBody class="remotes-view ui-card-section" density="compact">
            <SectionHeader title="Remotes" count={model.loaded() ? model.records().length : undefined}>
                <FilterInput
                    class="ui-card-section-filter"
                    value={model.filter()}
                    onInput={model.setFilter}
                    placeholder="Filter remotes"
                />
                <IconButton icon="rotate-right" label="Refresh" onClick={() => void model.refresh()} />
                <Button tone="accent" icon="plus" aria-expanded={adding()} onClick={() => setAdding(!adding())}>
                    Add remote
                </Button>
            </SectionHeader>
            <Show when={adding()}>
                <AddRemoteForm model={model} onDone={() => setAdding(false)} />
            </Show>
            <Show when={model.error()}>
                <Notice tone="danger">{model.error()}</Notice>
            </Show>
            <Show when={model.notice()}>
                <Notice tone="warning" onDismiss={() => model.setNotice("")}>
                    {model.notice()}
                </Notice>
            </Show>
            <PaneScroll class="remotes-list">
                <Show when={model.loaded()} fallback={<EmptyState loading>Loading…</EmptyState>}>
                    <Show when={groups().length > 0} fallback={<EmptyList filter={model.filter()} />}>
                        <For each={groups().map((g) => g.section)}>
                            {(section) => (
                                <Show when={groups().find((g) => g.section === section)}>
                                    {(group) => (
                                        <Section
                                            group={group()}
                                            byName={byName()}
                                            model={model}
                                            open={section !== "hidden" || hiddenOpen() || !!model.filter().trim()}
                                            onToggle={section === "hidden" ? () => setHiddenOpen(!hiddenOpen()) : undefined}
                                            onMenu={showMenu}
                                        />
                                    )}
                                </Show>
                            )}
                        </For>
                    </Show>
                </Show>
            </PaneScroll>
            <Show when={menu()}>
                {(m) => <ContextMenu items={m().items} x={m().x} y={m().y} onClose={() => setMenu(null)} />}
            </Show>
        </PaneBody>
    );
}

function EmptyList(props: { filter: string }): JSX.Element {
    return (
        <EmptyState>
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
        </EmptyState>
    );
}

function Section(props: {
    group: RemoteGroup;
    byName: Map<string, RemoteRecord>;
    model: RemotesViewModel;
    open: boolean;
    onToggle?: () => void;
    onMenu: (r: RemoteRecord, e: MouseEvent) => void;
}): JSX.Element {
    const names = createMemo(() => props.group.records.map((r) => r.name));
    return (
        <section class="remotes-section" data-section={props.group.section}>
            <Show
                when={props.onToggle}
                fallback={<SectionHeader variant="group" class="remotes-section-heading" title={props.group.label} />}
            >
                <SectionHeader
                    variant="group"
                    class="remotes-section-heading"
                    title={
                    <Button
                        tone="quiet"
                        class="remotes-section-toggle"
                        icon={props.open ? "chevron-down" : "chevron-right"}
                        aria-expanded={props.open}
                        onClick={() => props.onToggle?.()}
                    >
                        {props.group.label} ({props.group.records.length})
                    </Button>
                    }
                />
            </Show>
            <Show when={props.open}>
                <CardGrid
                    items={names()}
                    keyOf={(name) => name}
                    selectedKey={props.model.expanded()}
                    onClose={() => props.model.setExpanded(null)}
                    renderCard={(name) => (
                        <Show when={props.byName.get(name)}>
                            {(r) => <RemoteCard record={r()} model={props.model} onMenu={props.onMenu} />}
                        </Show>
                    )}
                    renderDetail={(name) => (
                        <Show when={props.byName.get(name)}>{(r) => <Detail record={r()} model={props.model} />}</Show>
                    )}
                />
            </Show>
        </section>
    );
}

function RemoteCard(props: {
    record: RemoteRecord;
    model: RemotesViewModel;
    onMenu: (r: RemoteRecord, e: MouseEvent) => void;
}): JSX.Element {
    const model = props.model;
    const r = () => props.record;
    const state = () => r().status.state || "disconnected";
    // The real name under a nickname, and the platform: "db1 · Linux · x86_64".
    const subtitle = createMemo(() => {
        const out: JSX.Element[] = [];
        if (displayName(r()) !== r().name) out.push(<span class="remotes-realname">{r().name}</span>);
        const platform = platformLabel(r().platform);
        if (platform) out.push(<span class="remotes-platform">{platform}</span>);
        return out.length > 0 ? out : undefined;
    });
    const activity = () => {
        const parts: string[] = [];
        if (r().sessions != null && r().sessions! > 0) parts.push(plural(r().sessions!, "session"));
        if (r().agents.length > 0) parts.push(plural(r().agents.length, "agent"));
        return parts.join(" · ");
    };
    const meta = createMemo(() => {
        const out: JSX.Element[] = [];
        const helper = helperLabel(r().helper);
        if (helper) out.push(<span class={`remotes-helper is-${r().helper.state}`}>{helper}</span>);
        if (activity()) out.push(<span class="remotes-activity">{activity()}</span>);
        return out;
    });
    const newTerminal = () => void model.run("New terminal", () => model.newTerminal(r().name));

    return (
        <Card
            class="remotes-card"
            data-remote={r().name}
            aria-label={`${displayName(r())}, ${statusLabel(r().status)}`}
            icon={<i class={r().kind === "wsl" ? "fa-brands fa-linux" : "fa fa-terminal"} />}
            title={displayName(r())}
            subtitle={subtitle()}
            meta={meta()}
            edgeColor={remoteColor(r())}
            status={{ tone: STATUS_TONES[state()] ?? "idle", label: statusLabel(r().status) }}
            attention={state() === "error"}
            muted={isHidden(r())}
            selected={model.expanded() === r().name}
            onSelect={() => model.toggleExpanded(r().name)}
            onOpen={newTerminal}
            onContextMenu={(e) => props.onMenu(r(), e)}
            actions={
                <>
                    <IconButton icon="terminal" label={`New terminal on ${r().name}`} onClick={newTerminal} />
                    <IconButton
                        icon="folder-open"
                        label={`Browse files on ${r().name}`}
                        onClick={() => void model.run("Browse files", () => model.browseFiles(r().name))}
                    />
                </>
            }
        />
    );
}

function Detail(props: { record: RemoteRecord; model: RemotesViewModel }): JSX.Element {
    const model = props.model;
    const r = () => props.record;
    const run = (what: string, action: () => Promise<void>) => () => void model.run(what, action);

    return (
        <div class="remotes-detail">
            <Show when={r().status.state === "error" && r().status.error}>
                <div class="remotes-detail-error">{r().status.error}</div>
            </Show>
            <DetailActions r={r} model={model} run={run} />
            <TestResult r={r} model={model} />
            <DetailSettings r={r} model={model} run={run} />
            <DetailAgents r={r} model={model} run={run} />
        </div>
    );
}

/** Add remote (§4.4): one field, the destination as you'd type it after
 *  `ssh`; a name, identity file and jump host under Advanced. Just a
 *  destination is saved in AgentMux's settings; an identity file or jump host
 *  is written to ~/.ssh/config, after the user confirms the exact block. */
function AddRemoteForm(props: { model: RemotesViewModel; onDone: () => void }): JSX.Element {
    const [destination, setDestination] = createSignal("");
    const [name, setName] = createSignal("");
    const [identityfile, setIdentityfile] = createSignal("");
    const [proxyjump, setProxyjump] = createSignal("");
    const [busy, setBusy] = createSignal(false);
    const [error, setError] = createSignal("");
    const parsed = () => parseDestination(destination());
    const writesSshConfig = () => !!(identityfile().trim() || proxyjump().trim());
    const submit = async (e: Event) => {
        e.preventDefault();
        const d = parsed();
        if (!d) {
            setError(destination().trim() ? "Type it as user@host:port (the port is optional)." : "Type the host to connect to.");
            return;
        }
        setBusy(true);
        setError("");
        try {
            const added = await props.model.addRemote(d, { name: name(), identityfile: identityfile(), proxyjump: proxyjump() });
            if (added) props.onDone();
        } catch (err) {
            setError(err instanceof Error ? err.message : String(err));
        } finally {
            setBusy(false);
        }
    };
    return (
        <form class="remotes-add" onSubmit={(e) => void submit(e)}>
            <TextInput
                class="remotes-add-destination"
                aria-label="Connect to"
                placeholder="user@host:port"
                value={destination()}
                onInput={(e) => setDestination(e.currentTarget.value)}
                ref={focusOnOpen}
            />
            <details class="remotes-add-more">
                <summary>Advanced</summary>
                <div class="remotes-settings">
                    <label class="remotes-setting">
                        <span>Name</span>
                        <TextInput
                            placeholder={(() => {
                                const d = parsed();
                                return d ? connectionName(d) : "user@host:port";
                            })()}
                            value={name()}
                            onInput={(e) => setName(e.currentTarget.value)}
                        />
                    </label>
                    <label class="remotes-setting">
                        <span>Identity file</span>
                        <TextInput
                            placeholder="~/.ssh/id_ed25519"
                            value={identityfile()}
                            onInput={(e) => setIdentityfile(e.currentTarget.value)}
                        />
                    </label>
                    <label class="remotes-setting" {...tip("An SSH server to hop through to reach this one (ssh's ProxyJump)")}>
                        <span>Connect through</span>
                        <TextInput placeholder="user@bastion" value={proxyjump()} onInput={(e) => setProxyjump(e.currentTarget.value)} />
                    </label>
                </div>
                <div class="remotes-add-note">
                    With an identity file or a host to connect through, the remote is added to the end of ~/.ssh/config (a copy of
                    the file is kept first), and you'll see exactly what is written before it is.
                </div>
            </details>
            <Show when={error()}>
                <div class="remotes-detail-error">{error()}</div>
            </Show>
            <div class="remotes-detail-actions">
                <Button type="submit" tone="accent" disabled={busy()}>
                    {busy() ? (writesSshConfig() ? "Waiting for you to confirm…" : "Adding…") : "Add"}
                </Button>
                <Button onClick={() => props.onDone()}>Cancel</Button>
            </div>
        </form>
    );
}

/** The right-click menu: the card's actions (§4.2). */
export function rowMenu(model: RemotesViewModel, r: RemoteRecord): ContextMenuItem[] {
    const act = (label: string, action: () => Promise<void>, extra: Partial<ContextMenuItem> = {}): ContextMenuItem => ({
        type: "action",
        label,
        onSelect: () => void model.run(label, action),
        ...extra,
    });
    const items: ContextMenuItem[] = [
        act("New terminal", () => model.newTerminal(r.name), { icon: MENU_ICON.openTerminal }),
        act("Browse files", () => model.browseFiles(r.name), { icon: MENU_ICON.openFolder }),
    ];
    if (r.kind === "ssh") items.push(act("Sessions…", () => model.showSessions(r.name), { icon: MENU_ICON.sessions }));
    const connection = connectionAction(r);
    if (connection) {
        items.push({ type: "separator" }, act(connection, () => runConnectionAction(model, r, connection), {
            icon: connection === "Connect" ? MENU_ICON.connect : MENU_ICON.disconnect,
        }));
    }
    items.push({ type: "separator" });
    items.push(
        isPinned(r) ? act("Unpin", () => model.setPinned(r.name, false), { icon: MENU_ICON.unpin }) : act("Pin", () => model.setPinned(r.name, true), { icon: MENU_ICON.pin }),
        isHidden(r) ? act("Unhide", () => model.setHidden(r.name, false), { icon: MENU_ICON.show }) : act("Hide", () => model.setHidden(r.name, true), { icon: MENU_ICON.hide })
    );
    if (r.sources.includes("recent")) items.push(act("Forget", () => model.forget(r.name), { icon: MENU_ICON.delete }));
    if (r.helper.installed) items.push(act("Remove helper", () => model.removeHelper(r.name), { icon: MENU_ICON.delete, danger: true }));
    items.push({ type: "separator" });
    items.push({
        type: "action",
        label: "Settings",
        icon: MENU_ICON.settings,
        onSelect: () => model.setExpanded(r.name),
    });
    return items;
}

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import { createStore } from "solid-js/store";

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { getApi, createBlock } from "@/store/global";
import { CORE_TOOLS, LOCAL_MODEL_TOOLS, cliCommandForPlatform, currentPlatform, rowIconClass, type CoreTool } from "@/app/store/toolchain-catalog";
import { EXTERNAL_WIDGETS, widgetCliCommandForPlatform } from "@/app/view/agent/providers/widget-catalog";
import { getProviderList } from "@/app/view/agent/providers";
import { checkProviderInstalled, invalidateInstallCheck, onInstallCheckInvalidated } from "@/app/view/agent/install-check-cache";
import { resolveDrift } from "@/app/view/agent/providers/version-drift";
import { ensureCapability, getCapability, isAvailable, watchCapability } from "@/app/store/toolchain-capabilities";
import { writeText as clipboardWriteText } from "@/util/clipboard";
import { SystemToolInstallInline } from "./SystemToolInstallInline";
import { CliDiskSpace } from "./CliDiskSpace";
import { toolNotes, toolStatus, widgetStatus } from "./toolchain-status";
import { WidgetPortField } from "./WidgetPortField";
import "./toolchain-view.scss";
import { Button, Card, CardGrid, EmptyState, FilterInput, IconButton, Pill, SectionHeader } from "@/app/element/ui";
import { matchesEveryWord } from "@/app/util/fuzzysearch";

// ── Port/localStorage helpers (unchanged from modal) ─────────────────────────

function loadWidgetPorts(): Record<string, number> {
    try {
        const raw = localStorage.getItem("agentmux:widget-ports");
        return raw ? (JSON.parse(raw) as Record<string, number>) : {};
    } catch {
        return {};
    }
}

function saveWidgetPort(widgetId: string, port: number | undefined): void {
    const saved = loadWidgetPorts();
    if (port === undefined) {
        delete saved[widgetId];
    } else {
        saved[widgetId] = port;
    }
    localStorage.setItem("agentmux:widget-ports", JSON.stringify(saved));
}

// ── Types (unchanged from modal) ─────────────────────────────────────────────

interface ToolRow {
    id: string;
    label: string;
    icon: string;
    /** Font Awesome brand icon name — core-tool rows only (provider/widget
     *  rows leave this unset). See `rowIconClass` in toolchain-catalog.ts. */
    brandIcon?: string;
    /** `local` = LOCAL_MODEL_TOOLS: same shape and PATH probe as `core`, but
     *  its own section and never eligible for the one-click installer. */
    kind: "core" | "local" | "provider";
    loading: boolean;
    found: boolean;
    version?: string;
    path?: string;
    source?: string;
    optional?: boolean;
    minVersion?: string;
    docsUrl?: string;
    installUrl?: string;
    installCommand?: string;
    npmPackage?: string;
    latestVersion?: string;
    /** `pinnedVersion` from the provider catalog — the version AgentMux
     *  validated. Provider rows only; core tools have no pin. */
    pinnedVersion?: string;
}

interface WidgetRow {
    id: string;
    label: string;
    icon: string;
    description: string;
    defaultPort: number;
    embedPath: string;
    healthCheckPath: string;
    docsUrl: string;
    installKind: "pip" | "npm" | "manual";
    installPkg?: string;
    cliLoading: boolean;
    cliFound: boolean;
    customPort?: number;
    healthLoading: boolean;
    running: boolean;
    statusCode?: number;
    /** Toolchain ids this widget needs (e.g. ["python"], ["docker"]) — see
     *  widget-catalog.ts's ExternalWidget.requires. */
    requires: string[];
}

interface ToolEnv {
    path: string;
    pathSource: string;
    os: string;
    arch: string;
}

function pathSourceLabel(src: string): string {
    switch (src) {
        case "login-shell":   return "from your login shell";
        case "fallback-dirs": return "from well-known dirs (login shell unavailable)";
        default:              return "inherited from the launching process";
    }
}

/**
 * Module-level so the registry lookup survives a pane close/reopen. Six hours
 * is well below the rate at which these CLIs ship and well above how often a
 * user opens this pane, so the automatic check costs approximately one request
 * per session rather than one per open.
 */
const LATEST_TTL_MS = 6 * 60 * 60 * 1000;
const latestCache: Record<string, string> = {};
let latestFetchedAt = 0;

// ── View component ────────────────────────────────────────────────────────────

/** `zoom`: the pane's zoom (`term:zoom`), applied to the whole view; 1 when not given. */
export function ToolchainView(props: { zoom?: () => number } = {}): JSX.Element {
    const plat = currentPlatform();
    const [env, setEnv] = createSignal<ToolEnv | null>(null);
    const [showPath, setShowPath] = createSignal(false);

    const toolRow = (t: CoreTool, kind: "core" | "local"): ToolRow => ({
        id: t.id, label: t.label, icon: t.icon, brandIcon: t.brandIcon, kind,
        loading: true, found: false,
        optional: t.optional, minVersion: t.minVersion,
        docsUrl: t.docsUrl, installUrl: t.installUrls[plat],
        installCommand: t.installCommand?.[plat],
    });
    const coreRows: ToolRow[] = CORE_TOOLS.map((t) => toolRow(t, "core"));
    const localRows: ToolRow[] = LOCAL_MODEL_TOOLS.map((t) => toolRow(t, "local"));
    const providerRows: ToolRow[] = getProviderList().map((p) => ({
        id: p.id, label: p.displayName, icon: p.icon, kind: "provider",
        loading: true, found: false, docsUrl: p.docsUrl, installUrl: p.docsUrl,
        npmPackage: p.npmPackage,
        // A CLI AgentMux doesn't install: the command of its own installer,
        // shown with a copy button while it is missing.
        installCommand: p.npmPackage ? undefined : (plat === "windows" ? p.windowsInstallCommand : p.unixInstallCommand) || undefined,
        // The version AgentMux validated. Core tools have no pin and stay
        // undefined — resolveDrift reports `unknown` for them rather than
        // claiming they're current.
        pinnedVersion: p.pinnedVersion || undefined,
    }));
    const [rows, setRows] = createStore<ToolRow[]>([...coreRows, ...localRows, ...providerRows]);

    const savedPorts = loadWidgetPorts();
    const [wrows, setWrows] = createStore<WidgetRow[]>(
        EXTERNAL_WIDGETS.map((w) => ({
            id: w.id, label: w.label, icon: w.icon,
            description: w.description, defaultPort: w.defaultPort,
            embedPath: w.embedPath, healthCheckPath: w.healthCheckPath,
            docsUrl: w.docsUrl, installKind: w.install.kind,
            installPkg: w.install.kind !== "manual" ? w.install.package : undefined,
            customPort: savedPorts[w.id],
            cliLoading: true, cliFound: false, healthLoading: false, running: false,
            requires: w.requires,
        }))
    );

    const probeWidget = async (idx: number) => {
        const w = EXTERNAL_WIDGETS[idx];
        const effectivePort = wrows[idx].customPort ?? w.defaultPort;
        const cliCmd = widgetCliCommandForPlatform(w, plat);
        if (!cliCmd) {
            setWrows(idx, { cliLoading: false, cliFound: false, healthLoading: true });
        } else {
            const data = { provider_id: w.id, cli_command: cliCmd, npm_package: "", pinned_version: "", windows_install_command: "", unix_install_command: "" };
            try {
                await RpcApi.ResolveCliCommand(TabRpcClient, data, { timeout: 10000 });
                setWrows(idx, { cliLoading: false, cliFound: true, healthLoading: true });
            } catch {
                setWrows(idx, { cliLoading: false, cliFound: false, healthLoading: true });
            }
        }
        try {
            const h = await RpcApi.WidgetHealthCommand(TabRpcClient, { port: effectivePort, health_check_path: w.healthCheckPath, health_check_body_contains: w.healthCheckBodyContains }, { timeout: 5000 });
            setWrows(idx, { healthLoading: false, running: h.healthy, statusCode: h.status_code ?? undefined });
        } catch {
            setWrows(idx, { healthLoading: false, running: false });
        }
    };

    // Rows whose catalog entry declares checkKind:"liveness" (currently just
    // docker) don't probe here directly — they go through the shared
    // toolchain-capabilities store (see the sync effect below) so this view
    // can never disagree with any other consumer (create-agent modal, launch
    // pre-flight) about whether the tool is actually available. See
    // docs/retro/RETRO_DOCKER_DETECTION_DIVERGENCE_2026_07_04.md.
    const probe = async (idx: number, opts?: { force?: boolean }) => {
        const row = rows[idx];
        const def = row.kind === "core" ? CORE_TOOLS.find((t) => t.id === row.id)
            : row.kind === "local" ? LOCAL_MODEL_TOOLS.find((t) => t.id === row.id)
            : getProviderList().find((p) => p.id === row.id);
        if (!def) { setRows(idx, { loading: false, found: false }); return; }
        if (row.kind === "core" && (def as any).checkKind === "liveness") {
            await ensureCapability(row.id, { force: opts?.force });
            return; // row update happens via the sync effect, from the shared store
        }
        if (row.kind === "provider") {
            // The agent picker's own check (`install.check`, one backend
            // lookup, one shared cache), so a card and its row never disagree.
            const prov = def as ReturnType<typeof getProviderList>[number];
            if (opts?.force) invalidateInstallCheck(prov.id);
            try {
                const r = await checkProviderInstalled({
                    providerId: prov.id,
                    cliCommand: prov.cliCommand,
                    npmPackage: prov.npmPackage || undefined,
                });
                setRows(idx, {
                    loading: false,
                    found: r.installed,
                    version: r.version ?? undefined,
                    path: r.path ?? undefined,
                    source: r.source ?? undefined,
                });
            } catch {
                setRows(idx, { loading: false, found: false });
            }
            return;
        }
        const cliCmd = cliCommandForPlatform(def as CoreTool, plat);
        const data = { provider_id: row.id, cli_command: cliCmd, npm_package: "", pinned_version: "", windows_install_command: "", unix_install_command: "" };
        try {
            const r = await RpcApi.ResolveCliCommand(TabRpcClient, data, { timeout: 12000 });
            setRows(idx, { loading: false, found: true, version: r.version && r.version !== "unknown" ? r.version : undefined, path: r.cli_path, source: r.source });
        } catch {
            setRows(idx, { loading: false, found: false });
        }
    };

    // Keep liveness-kind rows' visual state synced to the shared store,
    // including updates driven by this row's own background poll below
    // (watchCapability) — not just the initial probe.
    for (const [idx, row] of rows.entries()) {
        if (row.kind !== "core") continue;
        const def = CORE_TOOLS.find((t) => t.id === row.id);
        if (def?.checkKind !== "liveness") continue;
        createEffect(() => {
            const cap = getCapability(row.id);
            if (cap.status === "unknown") return; // not probed yet — keep the initial "Checking…" state
            setRows(idx, {
                loading: cap.status === "checking",
                found: cap.status === "available",
                version: cap.version,
                path: cap.path,
                source: cap.source,
            });
        });
    }

    onMount(() => {
        RpcApi.ToolchainEnvCommand(TabRpcClient, { timeout: 8000 }).then(setEnv).catch(() => setEnv(null));
        rows.forEach((_, i) => void probe(i));
        // A picker installed a CLI, or another view forgot an answer: re-ask
        // for the provider rows it concerns, so this pane never keeps a stale
        // "Not found" next to a card that says installed.
        onCleanup(
            onInstallCheckInvalidated((providerId) => {
                rows.forEach((r, i) => {
                    if (r.kind === "provider" && (providerId == null || r.id === providerId)) void probe(i);
                });
            }),
        );
        // Answer "am I behind?" without the user having to ask. This used to
        // require clicking "Check latest versions", so drift was invisible
        // unless you already suspected it — which is backwards for the one
        // question this pane exists to answer. Cached (see LATEST_TTL_MS) and
        // deliberately unawaited: rows render from the local installed-vs-pin
        // comparison immediately, and the upstream column fills in later or
        // not at all.
        void checkLatestVersions({ auto: true });
        wrows.forEach((_, i) => void probeWidget(i));
        // Background-poll liveness tools (docker) so this view self-heals —
        // e.g. reflects "user just started Docker Desktop" — within a few
        // seconds, with no manual refresh needed.
        for (const row of rows) {
            const def = row.kind === "core" ? CORE_TOOLS.find((t) => t.id === row.id) : undefined;
            if (def?.checkKind === "liveness") onCleanup(watchCapability(row.id));
        }
        // Probe every toolchain id any widget declares via `requires` so the
        // "Requires: X" hint below has a real answer, not just "unknown".
        // First real consumer of widget-catalog.ts's `requires` field.
        const requiredIds = new Set(wrows.flatMap((w) => w.requires));
        for (const id of requiredIds) void ensureCapability(id);
    });

    const [refreshing, setRefreshing] = createSignal(false);
    const refresh = async () => {
        setRefreshing(true);
        rows.forEach((_, i) => setRows(i, { loading: true, found: false, version: undefined }));
        wrows.forEach((_, i) => setWrows(i, { cliLoading: true, cliFound: false, healthLoading: false, running: false, statusCode: undefined }));
        try {
            await Promise.all([...rows.map((_, i) => probe(i, { force: true })), ...wrows.map((_, i) => probeWidget(i))]);
        } finally {
            setRefreshing(false);
        }
    };

    const [latestLoading, setLatestLoading] = createSignal(false);

    /** Per-row drift, from the one shared predicate. Cheap and pure — no memo
     *  per row, since `rows` is a store and this is called during render. */
    const rowDrift = (row: ToolRow) =>
        resolveDrift({
            installed: row.version,
            pinned: row.pinnedVersion,
            latest: row.latestVersion,
            found: row.found,
        });

    /** How many rows the user can actually act on. Drives the summary line, so
     *  the answer to "am I behind?" is available without scanning every row —
     *  deliberately counts `behind-pin` only, NOT `pin-behind-upstream`, which
     *  is ours to fix and must never be dressed up as the user's problem. */
    const behindCount = createMemo(
        () => rows.filter((r) => !r.loading && rowDrift(r).drift === "behind-pin").length
    );

    const checkLatestVersions = async (opts?: { auto?: boolean }) => {
        // An automatic check must not re-hit the registry every time the pane
        // is opened. A manual click always refetches — that is what the button
        // is for. Offline/failure leaves the cache empty and simply reports
        // `unknown` currency; it is never surfaced as an error, because having
        // no network is a normal state, not a fault (see
        // SPEC_PROVIDER_CLI_VERSION_UPGRADE_2026_09_06 §3 on `lookup-failed`).
        if (opts?.auto && Date.now() - latestFetchedAt < LATEST_TTL_MS) {
            if (Object.keys(latestCache).length) {
                rows.forEach((r, i) => {
                    if (r.npmPackage && latestCache[r.id]) setRows(i, "latestVersion", latestCache[r.id]);
                });
                return;
            }
        }
        const packages = rows
            .filter((r) => r.npmPackage)
            .map((r) => ({ id: r.id, package: r.npmPackage! }));
        if (!packages.length) return;
        setLatestLoading(true);
        try {
            const result = await RpcApi.ToolchainVersionsCommand(TabRpcClient, { packages }, { timeout: 15000 });
            rows.forEach((r, i) => {
                if (r.npmPackage && result[r.id] !== undefined) {
                    setRows(i, "latestVersion", result[r.id] ?? undefined);
                    if (result[r.id]) latestCache[r.id] = result[r.id]!;
                }
            });
            latestFetchedAt = Date.now();
        } catch {
            // network failure — leave latestVersion undefined
        } finally {
            setLatestLoading(false);
        }
    };

    const open = (url?: string) => { if (url) getApi().openExternal(url); };
    // Route through the CEF clipboard wrapper — navigator.clipboard is
    // blocked under CEF's Permissions-Policy. See SPEC_UNIFIED_CLIPBOARD_2026_05_18.md §3.3.
    const copy = (cmd?: string) => { if (cmd) void clipboardWriteText(cmd).catch(() => {}); };

    // Tool ids the backend's system-install catalog COULD cover
    // (system_install_handlers.rs) — this only ever ADDS a one-click
    // option alongside the existing "Install ↗" link/copy-command UI,
    // never replaces it. Even for these ids, the backend may still
    // resolve `available: false` on this specific machine (macOS without
    // brew, Linux without a recognized package manager) — in that case
    // `SystemToolInstallInline` renders nothing and the existing
    // installUrl button is the ONLY visible action, exactly as before
    // this feature existed. Codex P2, PR #2790 (an earlier version of
    // this file made the Install ↗ button itself conditionally stop
    // opening the URL for these ids, which was a dead end whenever the
    // backend couldn't resolve a command).
    const SYSTEM_INSTALLABLE_IDS = new Set(["git", "node", "npm", "python"]);
    const [expandedInstalls, setExpandedInstalls] = createSignal<ReadonlySet<string>>(new Set());
    const toggleInstallPanel = (id: string) => {
        setExpandedInstalls((prev) => {
            const next = new Set(prev);
            if (next.has(id)) next.delete(id); else next.add(id);
            return next;
        });
    };
    // Rows confirmed unavailable (no package manager detected/usable on
    // this machine) — hides the "or install it now" toggle for that row
    // once known, so a user who clicked it doesn't end up staring at a
    // permanently blank expanded area with no visible fallback (the
    // primary "Install ↗" button/link stays visible regardless either
    // way — this only hides the SECONDARY one-click toggle). Populated
    // reactively from SystemToolInstallInline's onUnavailable, so the
    // very first click can still briefly expand before collapsing —
    // this repo has no eager per-row pre-check today. reagent P2, PR #2790.
    const [unavailableInstalls, setUnavailableInstalls] = createSignal<ReadonlySet<string>>(new Set());
    const markUnavailable = (id: string) => {
        setExpandedInstalls((prev) => {
            const next = new Set(prev);
            next.delete(id);
            return next;
        });
        setUnavailableInstalls((prev) => new Set(prev).add(id));
    };

    // One card is open at a time, across every group.
    const [openKey, setOpenKey] = createSignal<string | null>(null);
    const toggleOpen = (key: string) => setOpenKey((cur) => (cur === key ? null : key));
    const [filter, setFilter] = createSignal("");

    const toolKey = (row: ToolRow) => `${row.kind}:${row.id}`;
    const widgetKey = (row: WidgetRow) => `widget:${row.id}`;
    const toolByKey = (key: string) => rows.find((r) => toolKey(r) === key);
    const widgetIndex = (key: string) => wrows.findIndex((w) => widgetKey(w) === key);

    const toolKeys = (kind: ToolRow["kind"]) =>
        rows
            .filter((r) => r.kind === kind && matchesEveryWord(filter(), r.label, r.id, r.version))
            .map(toolKey);
    const widgetKeys = () => wrows.filter((w) => matchesEveryWord(filter(), w.label, w.id, w.description)).map(widgetKey);
    const nothingMatches = () =>
        filter() !== "" &&
        toolKeys("core").length === 0 &&
        toolKeys("local").length === 0 &&
        toolKeys("provider").length === 0 &&
        widgetKeys().length === 0;

    const toolActions = (row: ToolRow): JSX.Element => (
        <>
            <Show when={!row.loading && !row.found && row.installUrl}>
                <Button density="compact" icon="arrow-up-right-from-square" onClick={() => open(row.installUrl)}>
                    Install
                </Button>
            </Show>
            <Show when={row.docsUrl}>
                <IconButton density="compact" icon="book" label="Docs" onClick={() => open(row.docsUrl)} />
            </Show>
        </>
    );

    const toolCard = (key: string): JSX.Element => (
        <Show when={toolByKey(key)}>
            {(row) => {
                const status = () => toolStatus(row(), rowDrift(row()));
                const notes = () => toolNotes(row(), rowDrift(row()));
                return (
                    <Card
                        class="toolchain-card"
                        data-tool={row().id}
                        aria-label={`${row().label}, ${status().label}`}
                        icon={<i class={rowIconClass(row().icon, row().brandIcon)} />}
                        title={row().label}
                        subtitle={status().subtitle}
                        meta={notes().map((n) => (
                            <span title={n.title}>{n.text}</span>
                        ))}
                        status={{ tone: status().tone, label: status().label }}
                        attention={status().attention}
                        muted={!row().loading && !row().found && row().optional}
                        selected={openKey() === key}
                        onSelect={() => toggleOpen(key)}
                        actions={toolActions(row())}
                    />
                );
            }}
        </Show>
    );

    const toolDetail = (key: string): JSX.Element => (
        <Show when={toolByKey(key)}>
            {(row) => (
                <div class="toolchain-detail">
                    <Show when={row().found && row().path}>
                        <span class="toolchain-detail-label">Path</span>
                        <code class="toolchain-detail-value">{row().path}</code>
                    </Show>
                    <For each={toolNotes(row(), rowDrift(row())).filter((n) => n.title)}>
                        {(n) => <p class="toolchain-detail-note">{n.title}</p>}
                    </For>
                    <Show when={row().loading}>
                        <p class="toolchain-detail-note">Looking for {row().label} on this machine…</p>
                    </Show>
                    <Show when={!row().loading && !row().found}>
                        <p class="toolchain-detail-note">Not found on this machine's PATH.</p>
                        <Show when={row().installCommand}>
                            <span class="toolchain-detail-label">Install command</span>
                            <div class="toolchain-detail-row">
                                <code class="toolchain-detail-value">{row().installCommand}</code>
                                <IconButton density="compact" icon="copy" label="Copy command" onClick={() => copy(row().installCommand)} />
                            </div>
                        </Show>
                        <Show when={row().kind === "core" && SYSTEM_INSTALLABLE_IDS.has(row().id) && !unavailableInstalls().has(row().id)}>
                            <div class="toolchain-detail-row">
                                <Button onClick={() => toggleInstallPanel(row().id)} pressed={expandedInstalls().has(row().id)}>
                                    Install it for me
                                </Button>
                            </div>
                        </Show>
                        <Show when={expandedInstalls().has(row().id)}>
                            <SystemToolInstallInline
                                toolId={row().id}
                                onInstalled={() => {
                                    toggleInstallPanel(row().id);
                                    const idx = rows.findIndex((r) => r.id === row().id);
                                    if (idx !== -1) void probe(idx, { force: true });
                                }}
                                onUnavailable={() => markUnavailable(row().id)}
                            />
                        </Show>
                    </Show>
                </div>
            )}
        </Show>
    );

    const effectivePort = (w: WidgetRow) => w.customPort ?? w.defaultPort;

    /** Use `port` for a widget, or its default when `port` is undefined, and look again. */
    const applyPort = (key: string, port: number | undefined) => {
        const idx = widgetIndex(key);
        if (idx === -1) return;
        const next = port === wrows[idx].defaultPort ? undefined : port;
        if (next === wrows[idx].customPort) return;
        setWrows(idx, { customPort: next, cliLoading: true, cliFound: false, healthLoading: false, running: false });
        saveWidgetPort(wrows[idx].id, next);
        void probeWidget(idx);
    };

    const widgetCard = (key: string): JSX.Element => (
        <Show when={wrows[widgetIndex(key)]}>
            {(row) => {
                const status = () => widgetStatus(row(), effectivePort(row()));
                return (
                    <Card
                        class="toolchain-card"
                        data-widget={row().id}
                        aria-label={`${row().label}, ${status().label}`}
                        icon={<i class={rowIconClass(row().icon)} />}
                        title={row().label}
                        subtitle={status().subtitle}
                        status={{ tone: status().tone, label: status().label }}
                        selected={openKey() === key}
                        onSelect={() => toggleOpen(key)}
                        actions={
                            <>
                                <Show when={row().running}>
                                    <Button
                                        density="compact"
                                        icon="arrow-up-right-from-square"
                                        onClick={() =>
                                            void createBlock({
                                                meta: {
                                                    view: "browser",
                                                    url: `http://127.0.0.1:${effectivePort(row())}${row().embedPath}`,
                                                    "frame:title": row().label,
                                                },
                                            })
                                        }
                                    >
                                        Open pane
                                    </Button>
                                </Show>
                                <Show when={row().docsUrl}>
                                    <IconButton density="compact" icon="book" label="Docs" onClick={() => open(row().docsUrl)} />
                                </Show>
                            </>
                        }
                    />
                );
            }}
        </Show>
    );

    const widgetDetail = (key: string): JSX.Element => (
        <Show when={wrows[widgetIndex(key)]}>
            {(row) => (
                <div class="toolchain-detail">
                    <p class="toolchain-detail-note">{row().description}</p>
                    <Show when={!row().running && row().requires.some((id) => !isAvailable(id))}>
                        <p class="toolchain-detail-note">
                            Requires: {row().requires.filter((id) => !isAvailable(id)).join(", ")}
                        </p>
                    </Show>
                    <WidgetPortField
                        port={effectivePort(row())}
                        defaultPort={row().defaultPort}
                        custom={row().customPort !== undefined}
                        onApply={(port) => applyPort(key, port)}
                    />
                </div>
            )}
        </Show>
    );

    const group = (title: string, keys: () => string[], card: (key: string) => JSX.Element, detail: (key: string) => JSX.Element) => (
        <Show when={keys().length > 0}>
            <SectionHeader variant="group" level={4} title={title} />
            <CardGrid
                ariaLabel={title}
                items={keys()}
                keyOf={(k) => k}
                renderCard={card}
                selectedKey={openKey()}
                renderDetail={detail}
                onClose={() => setOpenKey(null)}
            />
        </Show>
    );

    return (
        <div class="toolchain-view ui-density-compact" style={{ zoom: props.zoom?.() ?? 1 }}>
            <SectionHeader class="toolchain-view-header" title="Toolchain">
                {/* The one-glance answer to "am I behind?" — counts only what
                    the user can act on. A pin trailing upstream is ours and is
                    deliberately excluded, so this never nags about something
                    they cannot fix. */}
                <Show when={behindCount() > 0}>
                    <Pill tone="warn" icon="circle-up" title="Reinstall from each row to move to the version AgentMux validates against">
                        {behindCount()} {behindCount() === 1 ? "tool is" : "tools are"} behind
                    </Pill>
                </Show>
                <FilterInput class="ui-card-section-filter" value={filter()} onInput={setFilter} placeholder="Filter tools" />
                <Button tone="quiet" icon="rotate" busy={refreshing()} onClick={() => void refresh()}>
                    Refresh
                </Button>
                <Button
                    tone="quiet"
                    icon="arrow-up"
                    busy={latestLoading()}
                    onClick={() => void checkLatestVersions()}
                    title="Fetch latest published versions from npm registry"
                >
                    Check latest versions
                </Button>
            </SectionHeader>
            <div class="toolchain-body">
                <section class="toolchain-section">
                    <SectionHeader variant="group" level={4} title="Environment" />
                    <Show when={env()} fallback={<div class="toolchain-env-line">Loading…</div>}>
                        {(e) => (
                            <>
                                <div class="toolchain-env-line">
                                    <span class="toolchain-env-key">Platform</span>
                                    <span>{e().os} · {e().arch}</span>
                                </div>
                                <div class="toolchain-env-line">
                                    <span class="toolchain-env-key">PATH source</span>
                                    <Pill
                                        tone={
                                            e().pathSource === "login-shell"
                                                ? "ok"
                                                : e().pathSource === "inherited" && plat !== "windows"
                                                  ? "warn"
                                                  : "neutral"
                                        }
                                    >
                                        {pathSourceLabel(e().pathSource)}
                                    </Pill>
                                </div>
                                <Button tone="quiet" class="toolchain-path-toggle" onClick={() => setShowPath((v) => !v)}>
                                    {showPath() ? "Hide" : "Show"} effective PATH
                                </Button>
                                <Show when={showPath()}>
                                    <pre class="toolchain-path-dump">
                                        {e().path.split(e().os === "windows" ? ";" : ":").join("\n")}
                                    </pre>
                                </Show>
                            </>
                        )}
                    </Show>
                </section>

                <CliDiskSpace />

                {group("Core tools", () => toolKeys("core"), toolCard, toolDetail)}
                {group("Local models", () => toolKeys("local"), toolCard, toolDetail)}
                {group("Agent CLIs", () => toolKeys("provider"), toolCard, toolDetail)}
                {group("External widgets", widgetKeys, widgetCard, widgetDetail)}
                <Show when={nothingMatches()}>
                    <EmptyState>No tools match “{filter()}”.</EmptyState>
                </Show>
            </div>
        </div>
    );
}

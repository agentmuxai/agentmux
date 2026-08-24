// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import { createStore } from "solid-js/store";

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { getApi, createBlock } from "@/store/global";
import { CORE_TOOLS, cliCommandForPlatform, currentPlatform } from "@/app/view/agent/providers/toolchain-catalog";
import { EXTERNAL_WIDGETS, widgetCliCommandForPlatform } from "@/app/view/agent/providers/widget-catalog";
import { getProviderList } from "@/app/view/agent/providers";
import { ensureCapability, getCapability, isAvailable, watchCapability } from "@/app/store/toolchain-capabilities";
import { writeText as clipboardWriteText } from "@/util/clipboard";
import { SystemToolInstallInline } from "./SystemToolInstallInline";
import type { ToolchainViewModel } from "./toolchain-model";
import "./toolchain-view.scss";

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
    kind: "core" | "provider";
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

// ── View component ────────────────────────────────────────────────────────────

export function ToolchainView(_props: ViewComponentProps<ToolchainViewModel>): JSX.Element {
    const plat = currentPlatform();
    const [env, setEnv] = createSignal<ToolEnv | null>(null);
    const [showPath, setShowPath] = createSignal(false);

    const coreRows: ToolRow[] = CORE_TOOLS.map((t) => ({
        id: t.id, label: t.label, icon: t.icon, kind: "core",
        loading: true, found: false,
        optional: t.optional, minVersion: t.minVersion,
        docsUrl: t.docsUrl, installUrl: t.installUrls[plat],
        installCommand: t.installCommand?.[plat],
    }));
    const providerRows: ToolRow[] = getProviderList().map((p) => ({
        id: p.id, label: p.displayName, icon: p.icon, kind: "provider",
        loading: true, found: false, docsUrl: p.docsUrl, installUrl: p.docsUrl,
        npmPackage: p.npmPackage,
    }));
    const [rows, setRows] = createStore<ToolRow[]>([...coreRows, ...providerRows]);

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
        const def = row.kind === "core" ? CORE_TOOLS.find((t) => t.id === row.id) : getProviderList().find((p) => p.id === row.id);
        if (!def) { setRows(idx, { loading: false, found: false }); return; }
        if (row.kind === "core" && (def as any).checkKind === "liveness") {
            await ensureCapability(row.id, { force: opts?.force });
            return; // row update happens via the sync effect, from the shared store
        }
        const cliCmd = row.kind === "core" ? cliCommandForPlatform(def as any, plat) : (def as any).cliCommand;
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

    const refresh = () => {
        rows.forEach((_, i) => setRows(i, { loading: true, found: false, version: undefined }));
        rows.forEach((_, i) => void probe(i, { force: true }));
        wrows.forEach((_, i) => setWrows(i, { cliLoading: true, cliFound: false, healthLoading: false, running: false, statusCode: undefined }));
        wrows.forEach((_, i) => void probeWidget(i));
    };

    const [latestLoading, setLatestLoading] = createSignal(false);

    const checkLatestVersions = async () => {
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
                }
            });
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

    const renderRow = (row: ToolRow): JSX.Element => (
        <div class="toolchain-row" classList={{ "toolchain-row--missing": !row.loading && !row.found }}>
            <i class={`toolchain-row-icon fa-solid fa-${row.icon}`} aria-hidden="true" />
            <div class="toolchain-row-main">
                <div class="toolchain-row-title">
                    <span class="toolchain-row-name">{row.label}</span>
                    <Show when={row.loading}>
                        <span class="toolchain-pill toolchain-pill--muted">Checking…</span>
                    </Show>
                    <Show when={!row.loading && row.found}>
                        <span class="toolchain-pill toolchain-pill--ok">
                            <i class="fa-solid fa-check" /> {row.version ?? "installed"}
                        </span>
                        <Show when={row.source === "local_install"}>
                            <span class="toolchain-pill toolchain-pill--muted">managed</span>
                        </Show>
                        <Show when={row.latestVersion !== undefined && row.version !== undefined}>
                            <span
                                class={`toolchain-pill ${row.version === row.latestVersion ? "toolchain-pill--muted" : "toolchain-pill--update"}`}
                                title={`Latest: ${row.latestVersion}`}
                            >
                                <i class={`fa-solid ${row.version === row.latestVersion ? "fa-circle-check" : "fa-circle-up"}`} />
                                {" "}{row.version === row.latestVersion ? "up to date" : `${row.latestVersion} available`}
                            </span>
                        </Show>
                    </Show>
                    <Show when={!row.loading && !row.found}>
                        <span class="toolchain-pill" classList={{ "toolchain-pill--warn": !row.optional, "toolchain-pill--muted": row.optional }}>
                            {row.optional ? "Not installed (optional)" : "Not found"}
                        </span>
                    </Show>
                </div>
                <Show when={row.found && row.path}>
                    <div class="toolchain-row-path" title={row.path}>{row.path}</div>
                </Show>
                <Show when={!row.loading && !row.found && row.installCommand}>
                    <div class="toolchain-row-cmd">
                        <code>{row.installCommand}</code>
                        <button class="toolchain-link-btn" onClick={() => copy(row.installCommand)} title="Copy">
                            <i class="fa-solid fa-copy" />
                        </button>
                    </div>
                </Show>
                <Show when={!row.loading && !row.found && row.kind === "core" && SYSTEM_INSTALLABLE_IDS.has(row.id)}>
                    <button
                        type="button"
                        class="toolchain-link-btn toolchain-link-btn--install-now"
                        onClick={() => toggleInstallPanel(row.id)}
                    >
                        or install it now
                    </button>
                    <Show when={expandedInstalls().has(row.id)}>
                        <SystemToolInstallInline
                            toolId={row.id}
                            onInstalled={() => {
                                toggleInstallPanel(row.id);
                                const idx = rows.findIndex((r) => r.id === row.id);
                                if (idx !== -1) void probe(idx, { force: true });
                            }}
                        />
                    </Show>
                </Show>
            </div>
            <div class="toolchain-row-actions">
                <Show when={!row.loading && !row.found && row.installUrl}>
                    <button class="toolchain-btn" onClick={() => open(row.installUrl)}>
                        Install <i class="fa-solid fa-arrow-up-right-from-square" />
                    </button>
                </Show>
                <Show when={row.docsUrl}>
                    <button class="toolchain-link-btn" onClick={() => open(row.docsUrl)} title="Docs">
                        <i class="fa-solid fa-book" />
                    </button>
                </Show>
            </div>
        </div>
    );

    return (
        <div class="toolchain-view">
            <div class="toolchain-view-header">
                <button class="toolchain-btn toolchain-btn--ghost" onClick={refresh}>
                    <i class="fa-solid fa-rotate" /> Refresh
                </button>
                <button
                    class="toolchain-btn toolchain-btn--ghost"
                    onClick={() => void checkLatestVersions()}
                    disabled={latestLoading()}
                    title="Fetch latest published versions from npm registry"
                >
                    <i class={`fa-solid ${latestLoading() ? "fa-spinner fa-spin" : "fa-arrow-up"}`} />
                    {latestLoading() ? " Checking…" : " Check latest versions"}
                </button>
            </div>
            <div class="toolchain-body">
                <section class="toolchain-section">
                    <h3 class="toolchain-section-title">Environment</h3>
                    <Show when={env()} fallback={<div class="toolchain-env-line">Loading…</div>}>
                        {(e) => (
                            <>
                                <div class="toolchain-env-line">
                                    <span class="toolchain-env-key">Platform</span>
                                    <span>{e().os} · {e().arch}</span>
                                </div>
                                <div class="toolchain-env-line">
                                    <span class="toolchain-env-key">PATH source</span>
                                    <span class="toolchain-pill" classList={{
                                        "toolchain-pill--ok": e().pathSource === "login-shell",
                                        "toolchain-pill--warn": e().pathSource === "inherited" && plat !== "windows",
                                        "toolchain-pill--muted": e().pathSource === "fallback-dirs" || plat === "windows",
                                    }}>
                                        {pathSourceLabel(e().pathSource)}
                                    </span>
                                </div>
                                <button class="toolchain-link-btn toolchain-path-toggle" onClick={() => setShowPath((v) => !v)}>
                                    {showPath() ? "Hide" : "Show"} effective PATH
                                </button>
                                <Show when={showPath()}>
                                    <pre class="toolchain-path-dump">
                                        {e().path.split(e().os === "windows" ? ";" : ":").join("\n")}
                                    </pre>
                                </Show>
                            </>
                        )}
                    </Show>
                </section>

                <section class="toolchain-section">
                    <h3 class="toolchain-section-title">Core tools</h3>
                    <For each={rows.filter((r) => r.kind === "core")}>{renderRow}</For>
                </section>

                <section class="toolchain-section">
                    <h3 class="toolchain-section-title">Agent CLIs</h3>
                    <For each={rows.filter((r) => r.kind === "provider")}>{renderRow}</For>
                </section>

                <section class="toolchain-section">
                    <h3 class="toolchain-section-title">External Widgets</h3>
                    <For each={wrows}>
                        {(row, i) => (
                            <div class="toolchain-row" classList={{ "toolchain-row--missing": !row.cliLoading && !row.cliFound && !row.running }}>
                                <i class={`toolchain-row-icon fa-solid fa-${row.icon}`} aria-hidden="true" />
                                <div class="toolchain-row-main">
                                    <div class="toolchain-row-title">
                                        <span class="toolchain-row-name">{row.label}</span>
                                        <Show when={row.running}>
                                            <span class="toolchain-pill toolchain-pill--ok">
                                                <i class="fa-solid fa-circle" style="font-size:0.5em;vertical-align:middle" /> Running
                                            </span>
                                        </Show>
                                        <Show when={!row.cliLoading && row.cliFound && !row.running}>
                                            <span class="toolchain-pill toolchain-pill--muted">Installed</span>
                                        </Show>
                                        <Show when={row.cliLoading || row.healthLoading}>
                                            <span class="toolchain-pill toolchain-pill--muted">Checking…</span>
                                        </Show>
                                        <Show when={!row.cliLoading && !row.healthLoading && !row.cliFound && !row.running}>
                                            <span class="toolchain-pill toolchain-pill--muted">
                                                {row.installKind === "manual" ? "Not detected" : "Not installed"}
                                            </span>
                                        </Show>
                                    </div>
                                    <div class="toolchain-row-path toolchain-widget-desc">{row.description}</div>
                                    <Show when={!row.running && row.requires.some((id) => !isAvailable(id))}>
                                        <div class="toolchain-row-path">
                                            Requires: {row.requires.filter((id) => !isAvailable(id)).join(", ")}
                                        </div>
                                    </Show>
                                    <div class="toolchain-widget-port">
                                        <span class="toolchain-widget-port-label">Port</span>
                                        <input
                                            class="toolchain-widget-port-input"
                                            type="number" min="1" max="65535"
                                            value={row.customPort ?? row.defaultPort}
                                            onBlur={(e) => {
                                                const val = parseInt(e.currentTarget.value, 10);
                                                if (isNaN(val) || val <= 0 || val > 65535) { e.currentTarget.value = String(row.customPort ?? row.defaultPort); return; }
                                                const newPort = val === row.defaultPort ? undefined : val;
                                                setWrows(i(), { customPort: newPort, cliLoading: true, cliFound: false, healthLoading: false, running: false });
                                                saveWidgetPort(row.id, newPort);
                                                void probeWidget(i());
                                            }}
                                        />
                                        <Show when={row.customPort !== undefined}>
                                            <button class="toolchain-link-btn" title="Reset to default port"
                                                onClick={() => { setWrows(i(), { customPort: undefined, cliLoading: true, cliFound: false, healthLoading: false, running: false }); saveWidgetPort(row.id, undefined); void probeWidget(i()); }}>
                                                <i class="fa-solid fa-rotate-left" />
                                            </button>
                                        </Show>
                                    </div>
                                </div>
                                <div class="toolchain-row-actions">
                                    <Show when={row.running}>
                                        <button class="toolchain-btn"
                                            onClick={() => void createBlock({ meta: { view: "browser", url: `http://127.0.0.1:${row.customPort ?? row.defaultPort}${row.embedPath}`, "frame:title": row.label } })}>
                                            Open Pane <i class="fa-solid fa-arrow-up-right-from-square" />
                                        </button>
                                    </Show>
                                    <Show when={row.docsUrl}>
                                        <button class="toolchain-link-btn" onClick={() => open(row.docsUrl)} title="Docs">
                                            <i class="fa-solid fa-book" />
                                        </button>
                                    </Show>
                                </div>
                            </div>
                        )}
                    </For>
                </section>
            </div>
        </div>
    );
}

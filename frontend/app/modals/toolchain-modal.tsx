// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ToolchainModal — the hamburger ▸ "Toolchain" surface.
 *
 * Gives users visibility + control over the toolchain AgentMux runs CLIs in:
 *  - **Environment** — the effective PATH the srv resolves tools in, how it was
 *    derived (login-shell / fallback-dirs / inherited), and OS/arch. Makes the
 *    GUI-launch PATH problem (the "NPM failed" bug) diagnosable rather than
 *    mysterious.
 *  - **Core tools** — Node.js, npm, Git, Docker (versions + paths + status).
 *  - **Agent CLIs** — every provider, detected version + source.
 *
 * P1 (this file) is read-only: detection + install *links*. Install/repair in
 * place (reusing `install.start`) is P2. See
 * docs/specs/SPEC_TOOLCHAIN_MANAGER_2026-06-15.md.
 */

import { createSignal, For, onMount, Show, type JSX } from "solid-js";
import { createStore } from "solid-js/store";

import { Modal } from "@/element/modal";
import { openModal, type ModalCloseProps } from "@/app/store/modalmodel";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { getApi, createBlock } from "@/store/global";
import { getPlatform } from "@/util/platformutil";
import { CORE_TOOLS, cliCommandForPlatform, type Platform } from "@/app/view/agent/providers/toolchain-catalog";
import { EXTERNAL_WIDGETS, widgetCliCommandForPlatform } from "@/app/view/agent/providers/widget-catalog";
import { getProviderList } from "@/app/view/agent/providers";
import "./toolchain-modal.scss";

interface ToolRow {
    id: string;
    label: string;
    icon: string;
    kind: "core" | "provider";
    loading: boolean;
    found: boolean;
    version?: string;
    path?: string;
    /** "local_install" | "system_path" — where resolvecli found it. */
    source?: string;
    optional?: boolean;
    minVersion?: string;
    docsUrl?: string;
    installUrl?: string;
    installCommand?: string;
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
    /** CLI detection */
    cliLoading: boolean;
    cliFound: boolean;
    /** Health check (only run after cliFound or for manual installs) */
    healthLoading: boolean;
    running: boolean;
    statusCode?: number;
}

interface ToolEnv {
    path: string;
    pathSource: string;
    os: string;
    arch: string;
}

function platformKey(): Platform {
    switch (getPlatform()) {
        case "win32":
            return "windows";
        case "darwin":
            return "macos";
        default:
            return "linux";
    }
}

/** Human label for the PATH-source badge. */
function pathSourceLabel(src: string): string {
    switch (src) {
        case "login-shell":
            return "from your login shell";
        case "fallback-dirs":
            return "from well-known dirs (login shell unavailable)";
        default:
            return "inherited from the launching process";
    }
}

export const ToolchainModal = (props: ModalCloseProps): JSX.Element => {
    const plat = platformKey();
    const [env, setEnv] = createSignal<ToolEnv | null>(null);
    const [showPath, setShowPath] = createSignal(false);

    // Build the initial (loading) row list: core tools, then provider CLIs.
    const coreRows: ToolRow[] = CORE_TOOLS.map((t) => ({
        id: t.id,
        label: t.label,
        icon: t.icon,
        kind: "core",
        loading: true,
        found: false,
        optional: t.optional,
        minVersion: t.minVersion,
        docsUrl: t.docsUrl,
        installUrl: t.installUrls[plat],
        installCommand: t.installCommand?.[plat],
    }));
    const providerRows: ToolRow[] = getProviderList().map((p) => ({
        id: p.id,
        label: p.displayName,
        icon: p.icon,
        kind: "provider",
        loading: true,
        found: false,
        docsUrl: p.docsUrl,
        installUrl: p.docsUrl,
    }));

    const [rows, setRows] = createStore<ToolRow[]>([...coreRows, ...providerRows]);

    const widgetRows0: WidgetRow[] = EXTERNAL_WIDGETS.map((w) => ({
        id: w.id,
        label: w.label,
        icon: w.icon,
        description: w.description,
        defaultPort: w.defaultPort,
        embedPath: w.embedPath,
        healthCheckPath: w.healthCheckPath,
        docsUrl: w.docsUrl,
        installKind: w.install.kind,
        installPkg: w.install.kind !== "manual" ? w.install.package : undefined,
        cliLoading: true,
        cliFound: false,
        healthLoading: false,
        running: false,
    }));
    const [wrows, setWrows] = createStore<WidgetRow[]>(widgetRows0);

    const probeWidget = async (idx: number) => {
        const w = EXTERNAL_WIDGETS[idx];
        const cliCmd = widgetCliCommandForPlatform(w, plat);
        // For manual-install tools without a CLI command, skip CLI check and go
        // straight to health so the Running pill still lights up when they're live.
        if (!cliCmd) {
            setWrows(idx, { cliLoading: false, cliFound: false, healthLoading: true });
        } else {
            const data = {
                provider_id: w.id,
                cli_command: cliCmd,
                npm_package: "",
                pinned_version: "",
                windows_install_command: "",
                unix_install_command: "",
            };
            try {
                await RpcApi.ResolveCliCommand(TabRpcClient, data, { timeout: 10000 });
                setWrows(idx, { cliLoading: false, cliFound: true, healthLoading: true });
            } catch {
                setWrows(idx, { cliLoading: false, cliFound: false, healthLoading: true });
            }
        }
        // Always run the health check regardless of CLI result — user may have
        // started the server manually even without the CLI on PATH.
        try {
            const h = await RpcApi.WidgetHealthCommand(
                TabRpcClient,
                {
                    port: w.defaultPort,
                    health_check_path: w.healthCheckPath,
                    health_check_body_contains: w.healthCheckBodyContains,
                },
                { timeout: 5000 }
            );
            setWrows(idx, { healthLoading: false, running: h.healthy, statusCode: h.status_code ?? undefined });
        } catch {
            setWrows(idx, { healthLoading: false, running: false });
        }
    };

    const probe = async (idx: number) => {
        const row = rows[idx];
        const def =
            row.kind === "core"
                ? CORE_TOOLS.find((t) => t.id === row.id)
                : getProviderList().find((p) => p.id === row.id);
        if (!def) {
            setRows(idx, { loading: false, found: false });
            return;
        }
        // Detection MUST be pure — `resolvecli` is resolve-OR-install: with a
        // non-empty `npm_package` it `npm install`s any provider not found in
        // the versioned dir. We pass an EMPTY npm_package so it only probes the
        // versioned install dir then the system PATH (never installs). Install
        // is an explicit, user-initiated action (P2). See SPEC_TOOLCHAIN_MANAGER.
        const cliCmd =
            row.kind === "core"
                ? cliCommandForPlatform(def as any, plat)
                : (def as any).cliCommand;
        const data = {
            provider_id: row.id,
            cli_command: cliCmd,
            npm_package: "",
            pinned_version: "",
            windows_install_command: "",
            unix_install_command: "",
        };
        try {
            const r = await RpcApi.ResolveCliCommand(TabRpcClient, data, { timeout: 12000 });
            setRows(idx, {
                loading: false,
                found: true,
                version: r.version && r.version !== "unknown" ? r.version : undefined,
                path: r.cli_path,
                source: r.source,
            });
        } catch {
            // resolvecli throws when the tool is found nowhere.
            setRows(idx, { loading: false, found: false });
        }
    };

    onMount(() => {
        RpcApi.ToolchainEnvCommand(TabRpcClient, { timeout: 8000 })
            .then(setEnv)
            .catch(() => setEnv(null));
        // Probe every row in parallel — each updates its own store entry.
        rows.forEach((_, i) => void probe(i));
        wrows.forEach((_, i) => void probeWidget(i));
    });

    const refresh = () => {
        rows.forEach((_, i) => setRows(i, { loading: true, found: false, version: undefined }));
        rows.forEach((_, i) => void probe(i));
        wrows.forEach((_, i) =>
            setWrows(i, { cliLoading: true, cliFound: false, healthLoading: false, running: false, statusCode: undefined })
        );
        wrows.forEach((_, i) => void probeWidget(i));
    };

    const open = (url?: string) => {
        if (url) getApi().openExternal(url);
    };
    const copy = (cmd?: string) => {
        if (cmd) void navigator.clipboard?.writeText(cmd);
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
                    </Show>
                    <Show when={!row.loading && !row.found}>
                        <span
                            class="toolchain-pill"
                            classList={{
                                "toolchain-pill--warn": !row.optional,
                                "toolchain-pill--muted": row.optional,
                            }}
                        >
                            {row.optional ? "Not installed (optional)" : "Not found"}
                        </span>
                    </Show>
                </div>
                <Show when={row.found && row.path}>
                    <div class="toolchain-row-path" title={row.path}>
                        {row.path}
                    </div>
                </Show>
                <Show when={!row.loading && !row.found && row.installCommand}>
                    <div class="toolchain-row-cmd">
                        <code>{row.installCommand}</code>
                        <button class="toolchain-link-btn" onClick={() => copy(row.installCommand)} title="Copy">
                            <i class="fa-solid fa-copy" />
                        </button>
                    </div>
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
        <Modal open={true} onClose={props.close} scope="window" size="lg">
            <div class="modal-panel-header toolchain-header">
                <div class="modal-panel-title">
                    <i class="fa-solid fa-wrench" /> Toolchain
                </div>
                <button class="toolchain-btn toolchain-btn--ghost" onClick={refresh}>
                    <i class="fa-solid fa-rotate" /> Refresh
                </button>
            </div>
            <div class="modal-panel-body toolchain-body">
                {/* Environment */}
                <section class="toolchain-section">
                    <h3 class="toolchain-section-title">Environment</h3>
                    <Show when={env()} fallback={<div class="toolchain-env-line">Loading…</div>}>
                        {(e) => (
                            <>
                                <div class="toolchain-env-line">
                                    <span class="toolchain-env-key">Platform</span>
                                    <span>
                                        {e().os} · {e().arch}
                                    </span>
                                </div>
                                <div class="toolchain-env-line">
                                    <span class="toolchain-env-key">PATH source</span>
                                    <span
                                        class="toolchain-pill"
                                        classList={{
                                            "toolchain-pill--ok": e().pathSource === "login-shell",
                                            "toolchain-pill--warn":
                                                e().pathSource === "inherited" &&
                                                plat !== "windows",
                                            "toolchain-pill--muted":
                                                e().pathSource === "fallback-dirs" ||
                                                plat === "windows",
                                        }}
                                    >
                                        {pathSourceLabel(e().pathSource)}
                                    </span>
                                </div>
                                <button
                                    class="toolchain-link-btn toolchain-path-toggle"
                                    onClick={() => setShowPath((v) => !v)}
                                >
                                    {showPath() ? "Hide" : "Show"} effective PATH
                                </button>
                                <Show when={showPath()}>
                                    <pre class="toolchain-path-dump">
                                        {e()
                                            .path.split(e().os === "windows" ? ";" : ":")
                                            .join("\n")}
                                    </pre>
                                </Show>
                            </>
                        )}
                    </Show>
                </section>

                {/* Core tools */}
                <section class="toolchain-section">
                    <h3 class="toolchain-section-title">Core tools</h3>
                    <For each={rows.filter((r) => r.kind === "core")}>{renderRow}</For>
                </section>

                {/* Agent CLIs */}
                <section class="toolchain-section">
                    <h3 class="toolchain-section-title">Agent CLIs</h3>
                    <For each={rows.filter((r) => r.kind === "provider")}>{renderRow}</For>
                </section>

                {/* External Widgets */}
                <section class="toolchain-section">
                    <h3 class="toolchain-section-title">External Widgets</h3>
                    <For each={wrows}>
                        {(row) => (
                            <div class="toolchain-row" classList={{ "toolchain-row--missing": !row.cliLoading && !row.cliFound && !row.running }}>
                                <i class={`toolchain-row-icon fa-solid fa-${row.icon}`} aria-hidden="true" />
                                <div class="toolchain-row-main">
                                    <div class="toolchain-row-title">
                                        <span class="toolchain-row-name">{row.label}</span>
                                        {/* Running pill — shown first when healthy */}
                                        <Show when={row.running}>
                                            <span class="toolchain-pill toolchain-pill--ok">
                                                <i class="fa-solid fa-circle" style="font-size:0.5em;vertical-align:middle" /> Running
                                            </span>
                                        </Show>
                                        {/* CLI detected pill */}
                                        <Show when={!row.cliLoading && row.cliFound && !row.running}>
                                            <span class="toolchain-pill toolchain-pill--muted">Installed</span>
                                        </Show>
                                        {/* Loading state */}
                                        <Show when={row.cliLoading || row.healthLoading}>
                                            <span class="toolchain-pill toolchain-pill--muted">Checking…</span>
                                        </Show>
                                        {/* Not found */}
                                        <Show when={!row.cliLoading && !row.healthLoading && !row.cliFound && !row.running}>
                                            <span class="toolchain-pill toolchain-pill--muted">
                                                {row.installKind === "manual" ? "Not detected" : "Not installed"}
                                            </span>
                                        </Show>
                                    </div>
                                    <div class="toolchain-row-path toolchain-widget-desc">{row.description}</div>
                                </div>
                                <div class="toolchain-row-actions">
                                    <Show when={row.running}>
                                        <button
                                            class="toolchain-btn"
                                            onClick={() => {
                                                void createBlock({
                                                    meta: {
                                                        view: "browser",
                                                        url: `http://127.0.0.1:${row.defaultPort}${row.embedPath}`,
                                                        "frame:title": row.label,
                                                    },
                                                });
                                                props.close();
                                            }}
                                        >
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
            <div class="modal-panel-footer">
                <button class="modal-btn modal-btn--confirm" data-modal-dismiss onClick={() => props.close()}>
                    Close
                </button>
            </div>
        </Modal>
    );
};

/** Hamburger entry point. */
export function openToolchainModal(): void {
    openModal(ToolchainModal);
}

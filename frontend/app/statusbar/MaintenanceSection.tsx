// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import {
    appUpdate,
    availableRelease,
    cancelUpgrade,
    checkedAt,
    checkForUpdate,
    loadAppUpdate,
    releaseDownload,
    startAppUpdateDownload,
    upgradeAndRestart,
    upgradeUnderWay,
} from "@/store/app-update";
import { atoms, getApi, openLink } from "@/store/global";
import { WpsEvent } from "@/store/mps-events";
import { createEffect, createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import "./_maintenance-section.scss";
import { Button } from "@/app/element/ui";

// ── Types ────────────────────────────────────────────────────────────────────

type MigStep = {
    id: string;
    label: string;
    status: "running" | "done";
    durationMs: number | null;
    startedAtMs: number;
};

type MigState =
    | { kind: "idle"; pendingCount: number }
    | { kind: "running"; steps: MigStep[] }
    | { kind: "complete"; steps: MigStep[]; applied: number }
    | { kind: "failed"; steps: MigStep[]; error: string; failedId: string | null };

type VacState =
    | { kind: "idle"; lastRunMs: number | null }
    | { kind: "running" }
    | { kind: "done"; rowsDeleted: number; ranAtMs: number };

// ── Helpers ──────────────────────────────────────────────────────────────────

function fmtDate(ms: number): string {
    return new Date(ms).toLocaleDateString("en-US", { month: "short", day: "numeric" });
}

function elapsedSecs(startedAtMs: number, nowMs: number): string {
    return `${((nowMs - startedAtMs) / 1000).toFixed(1)}s`;
}

function fmtDuration(ms: number): string {
    return `${(ms / 1000).toFixed(1)}s`;
}

// ── Component ────────────────────────────────────────────────────────────────

export const MaintenanceSection = (): JSX.Element => {
    // A newer release srv found (store/app-update.ts) leads; the host
    // updater's own states are kept for a host that has one.
    onMount(() => void loadAppUpdate());
    const release = () => availableRelease();
    const download = () => (release() ? releaseDownload() : null);
    const updaterStatus = () => {
        if (!release()) return atoms.updaterStatusAtom();
        const d = download()?.state;
        if (d === "downloading" || d === "unpacking") return "downloading";
        if (d === "ready" || d === "waiting" || d === "restarting") return "ready";
        if (d === "failed") return "error";
        return "available";
    };
    const percent = () => {
        const d = download();
        return d && d.total > 0 ? Math.floor((d.received * 100) / d.total) : 0;
    };
    const [startError, setStartError] = createSignal<string | null>(null);
    const [checking, setChecking] = createSignal(false);
    const [checkError, setCheckError] = createSignal<string | null>(null);
    const handleCheck = () => {
        setChecking(true);
        setCheckError(null);
        checkForUpdate()
            .catch((e) => setCheckError(String(e?.message ?? e)))
            .finally(() => setChecking(false));
    };
    // Where this AgentMux stands, beside Check for update. Nothing checks on
    // its own, so before the first check it says so.
    const standing = () => {
        const s = appUpdate();
        const checked = s && s.checked_ms > 0 ? checkedAt(s.checked_ms) : null;
        if (s?.state === "local-build" && s.latest) return `Development build. The latest release is v${s.latest}.`;
        if (s?.state === "up-to-date") return `v${s.current} is up to date${checked ? ` (checked ${checked})` : ""}`;
        if (s?.state === "available" && checked) return `Checked ${checked}`;
        return "Not checked for updates yet";
    };
    const updaterVersion = () => release()?.version ?? atoms.updaterVersionAtom();
    const [showNotes, setShowNotes] = createSignal(false);

    const [migState, setMigState] = createSignal<MigState>({ kind: "idle", pendingCount: 0 });
    const [vacState, setVacState] = createSignal<VacState>({ kind: "idle", lastRunMs: null });
    const [nowMs, setNowMs] = createSignal(Date.now());

    // Fetch pending_migrations count from backend info on mount
    onMount(() => {
        getApi().getBackendInfo().then((info) => {
            const pending = info?.pending_migrations ?? 0;
            setMigState({ kind: "idle", pendingCount: pending });
        }).catch(() => {});
    });

    // Timer — ticks every 500ms only while a migration is running to drive elapsed timers.
    onMount(() => {
        const id = setInterval(() => {
            if (migState().kind === "running") {
                setNowMs(Date.now());
            }
        }, 500);
        onCleanup(() => clearInterval(id));
    });

    // Subscribe to migration progress events (from CEF, not MPS WebSocket)
    onMount(() => {
        let unlisten: (() => void) | null = null;
        getApi().listen(WpsEvent.UpgradeMigrationEvent, (payload: any) => {
            const kind: string = payload?.kind ?? "";
            if (kind === "start") {
                const id: string = payload.id ?? "";
                const label: string = payload.label ?? id;
                setMigState((prev) => {
                    const steps: MigStep[] = prev.kind === "running" ? [...prev.steps] : [];
                    steps.push({ id, label, status: "running", durationMs: null, startedAtMs: Date.now() });
                    return { kind: "running", steps };
                });
            } else if (kind === "done") {
                const id: string = payload.id ?? "";
                const durationMs: number | null = payload.duration_ms ?? null;
                setMigState((prev) => {
                    if (prev.kind !== "running") return prev;
                    const steps = prev.steps.map((s) =>
                        s.id === id ? { ...s, status: "done" as const, durationMs } : s
                    );
                    return { ...prev, steps };
                });
            } else if (kind === "complete") {
                const applied: number = payload.applied ?? 0;
                setMigState((prev) => {
                    const steps = prev.kind === "running" ? prev.steps : [];
                    return { kind: "complete", steps, applied };
                });
            }
        }).then((fn) => { unlisten = fn; });

        onCleanup(() => unlisten?.());
    });

    onMount(() => {
        let unlisten: (() => void) | null = null;
        getApi().listen(WpsEvent.UpgradeMigrationsComplete, (payload: any) => {
            const applied: number = payload?.applied ?? 0;
            setMigState((prev) => {
                // migration-event kind:"complete" may have already transitioned state;
                // preserve it — don't reset steps/applied with this fallback event.
                if (prev.kind !== "running") return prev;
                return { kind: "complete", steps: prev.steps, applied };
            });
        }).then((fn) => { unlisten = fn; });

        onCleanup(() => unlisten?.());
    });

    onMount(() => {
        let unlisten: (() => void) | null = null;
        getApi().listen(WpsEvent.UpgradeMigrationsFailed, (payload: any) => {
            const error: string = payload?.error ?? "Migration failed";
            const failedId: string | null = payload?.failedId ?? null;
            setMigState((prev) => {
                const steps = prev.kind === "running" ? prev.steps : [];
                return { kind: "failed", steps, error, failedId };
            });
        }).then((fn) => { unlisten = fn; });

        onCleanup(() => unlisten?.());
    });

    onMount(() => {
        let unlisten: (() => void) | null = null;
        getApi().listen(WpsEvent.UpgradeSagaVacuumDone, (payload: any) => {
            const rowsDeleted: number = payload?.rows_deleted ?? 0;
            const ranAtMs = Date.now();
            setVacState({ kind: "done", rowsDeleted, ranAtMs });
            setTimeout(() => {
                setVacState({ kind: "idle", lastRunMs: ranAtMs });
            }, 3000);
        }).then((fn) => { unlisten = fn; });

        onCleanup(() => unlisten?.());
    });

    // ── Actions ───────────────────────────────────────────────────────────────

    const handleRunMigrations = async () => {
        if (migState().kind === "running") return;
        setMigState({ kind: "running", steps: [] });
        try {
            await getApi().runMigrations();
        } catch (e: any) {
            setMigState({ kind: "failed", steps: [], error: String(e?.message ?? e), failedId: null });
        }
    };

    const handleRunVacuum = async () => {
        if (vacState().kind === "running") return;
        setVacState({ kind: "running" });
        try {
            await getApi().runSagaVacuum();
        } catch {
            setVacState({ kind: "idle", lastRunMs: null });
        }
    };

    const runUpgrade = (now: boolean) => {
        setStartError(null);
        upgradeAndRestart(now).catch((e) => setStartError(String(e?.message ?? e)));
    };

    const handleInstallUpdate = () => {
        const r = release();
        if (r) {
            // srv downloads, verifies and unpacks what it can (a Windows
            // portable); for any other install the release page is the way.
            if (!appUpdate()?.can_download) {
                void openLink(r.url);
                return;
            }
            setStartError(null);
            startAppUpdateDownload().catch((e) => setStartError(String(e?.message ?? e)));
            return;
        }
        getApi().installAppUpdate();
    };

    // ── Derived ───────────────────────────────────────────────────────────────

    const badge = (): string | null => {
        const mig = migState();
        if (mig.kind === "failed") return `✗ failed`;
        if (mig.kind === "idle" && mig.pendingCount > 0) return `⚠ ${mig.pendingCount} pending`;
        if (mig.kind === "running") return "·running";
        const upd = updaterStatus();
        if (upd === "available") {
            const ver = updaterVersion();
            return ver ? `↓ ${ver}` : "↓ update";
        }
        if (upd === "downloading") return "↓ …";
        if (upd === "ready") return "↑ ready";
        return null;
    };

    // ── Render ────────────────────────────────────────────────────────────────

    return (
        <>
            <div class="instance-panel-section-title">
                Maintenance
                <Show when={badge() != null}>
                    <span class={`maintenance-badge${migState().kind === "failed" ? " maintenance-badge--error" : ""}`}>{badge()}</span>
                </Show>
            </div>

            {/* ── Update row ─────────────────────────────────────────────── */}
            <Show when={updaterStatus() !== "up-to-date" && updaterStatus() !== "checking"}>
                <div class="maintenance-update-row">
                    <Show when={updaterStatus() === "available"}>
                        <span class="maintenance-icon maintenance-icon--update">↓</span>
                        <span class="maintenance-row-text">
                            {updaterVersion() ? `v${updaterVersion()} is out` : "Update available"}
                        </span>
                        <Show when={release() && appUpdate()?.notes}>
                            <Button
                                density="compact"
                                class="maintenance-btn"
                                aria-expanded={showNotes()}
                                onClick={() => setShowNotes(!showNotes())}
                            >
                                What's new
                            </Button>
                        </Show>
                        <Button
                            density="compact"
                            tone="accent"
                            class="maintenance-btn maintenance-btn--primary"
                            onClick={handleInstallUpdate}
                        >
                            Download
                        </Button>
                    </Show>
                    <Show when={updaterStatus() === "downloading"}>
                        <span class="maintenance-icon maintenance-icon--working">↓</span>
                        <span class="maintenance-row-text maintenance-row-text--dim">
                            {download()?.state === "unpacking"
                                ? `Unpacking v${updaterVersion()}…`
                                : download()
                                  ? `Downloading v${updaterVersion()}… ${percent()}%`
                                  : "Downloading…"}
                        </span>
                    </Show>
                    {/* Staged by srv: restart into it, once agents finish their turn. */}
                    <Show when={download()?.state === "ready" && (download()?.folder || download()?.installer)}>
                        <span class="maintenance-icon maintenance-icon--update">↑</span>
                        <span class="maintenance-row-text" title={download()?.folder ?? download()?.installer}>
                            v{updaterVersion()} is ready
                        </span>
                        <Show when={download()?.folder}>
                            <Button
                                density="compact"
                                class="maintenance-btn"
                                onClick={() => getApi().openNativePath(download()!.folder!)}
                            >
                                Show folder
                            </Button>
                        </Show>
                        {/* An installer (or an AppImage's next version) is shown in its folder,
                            never opened: that would run it. */}
                        <Show when={download()?.installer}>
                            <Button
                                density="compact"
                                class="maintenance-btn"
                                onClick={() => getApi().revealInFileExplorer(download()!.installer!)}
                            >
                                {appUpdate()?.install === "linux-appimage" ? "Show file" : "Show installer"}
                            </Button>
                        </Show>
                        <Button
                            density="compact"
                            tone="accent"
                            class="maintenance-btn maintenance-btn--primary"
                            onClick={() => runUpgrade(false)}
                        >
                            Upgrade &amp; restart
                        </Button>
                    </Show>
                    <Show when={download()?.state === "waiting"}>
                        <span class="maintenance-icon maintenance-icon--working">↑</span>
                        <span class="maintenance-row-text maintenance-row-text--dim">
                            {download()!.waiting_for > 0
                                ? `Waiting for ${download()!.waiting_for} agent${download()!.waiting_for === 1 ? "" : "s"} to finish…`
                                : "Restarting soon…"}
                        </span>
                        <Button density="compact" class="maintenance-btn" onClick={() => runUpgrade(true)}>
                            Restart now
                        </Button>
                        <Button density="compact" class="maintenance-btn" onClick={() => cancelUpgrade().catch((e) => setStartError(String(e?.message ?? e)))}>
                            Cancel
                        </Button>
                    </Show>
                    <Show when={download()?.state === "restarting"}>
                        <span class="maintenance-icon maintenance-icon--working">↑</span>
                        {/* Only the launcher's dialog can stop it now: No there cancels. */}
                        <span class="maintenance-row-text maintenance-row-text--dim">
                            Restarting: answer the AgentMux dialog (No cancels)
                        </span>
                    </Show>
                    <Show when={updaterStatus() === "ready" && !download()}>
                        <span class="maintenance-icon maintenance-icon--update">↑</span>
                        <span class="maintenance-row-text">
                            {updaterVersion() ? `v${updaterVersion()} ready` : "Ready to install"}
                        </span>
                        <Button
                            density="compact"
                            tone="accent"
                            class="maintenance-btn maintenance-btn--primary"
                            onClick={handleInstallUpdate}
                        >
                            Restart
                        </Button>
                    </Show>
                    <Show when={updaterStatus() === "error"}>
                        <span class="maintenance-icon maintenance-icon--error">✗</span>
                        <span class="maintenance-row-text maintenance-row-text--error" title={download()?.error}>
                            {download()?.error ? `Download failed: ${download()!.error}` : "Update failed"}
                        </span>
                        <Show when={download()}>
                            <Button density="compact" class="maintenance-btn" onClick={handleInstallUpdate}>
                                Try again
                            </Button>
                            <Button density="compact" class="maintenance-btn" onClick={() => void openLink(release()!.url)}>
                                Release page
                            </Button>
                        </Show>
                    </Show>
                </div>
            </Show>

            {/* What agents left running that the restart stops (srv's background_work). */}
            <Show when={(download()?.state === "waiting" || download()?.state === "restarting") && (download()?.stopping?.length ?? 0) > 0}>
                <div class="maintenance-row-text maintenance-row-text--warn">The restart also stops what agents left running:</div>
                <For each={download()!.stopping}>
                    {(line) => <div class="maintenance-row-text maintenance-row-text--dim">{line}</div>}
                </For>
            </Show>
            <Show when={download()?.state === "ready" && download()?.error}>
                <div class="maintenance-row-text maintenance-row-text--error">{download()!.error}</div>
            </Show>
            <Show when={startError()}>
                <div class="maintenance-row-text maintenance-row-text--error">{startError()}</div>
            </Show>
            <Show when={showNotes() && release() && appUpdate()?.notes}>
                <pre class="maintenance-release-notes">{appUpdate()?.notes}</pre>
            </Show>
            {/* Check for update, always: nothing checks on its own, and a new
                check replaces the release on offer (not while a download or
                restart into it is under way). A `task dev` build, which
                doesn't upgrade, learns which release is latest. */}
            <div class="maintenance-update-row">
                <span class="maintenance-row-text maintenance-row-text--dim">{standing()}</span>
                <Show when={appUpdate()?.state === "local-build" && appUpdate()?.url}>
                    <Button density="compact" class="maintenance-btn" onClick={() => void openLink(appUpdate()!.url!)}>
                        View
                    </Button>
                </Show>
                <Button
                    density="compact"
                    class="maintenance-btn"
                    disabled={checking() || upgradeUnderWay()}
                    onClick={handleCheck}
                >
                    {checking() ? "Checking…" : release() ? "Check again" : "Check for update"}
                </Button>
            </div>
            <Show when={checkError()}>
                <div class="maintenance-row-text maintenance-row-text--error">{checkError()}</div>
            </Show>

            {/* ── Migration rows ─────────────────────────────────────────── */}
            <Show when={migState().kind === "idle" && (migState() as Extract<MigState, {kind: "idle"}>).pendingCount > 0}>
                <div class="maintenance-alert-row">
                    <span class="maintenance-icon maintenance-icon--warn">⚠</span>
                    <span class="maintenance-row-text maintenance-row-text--warn">
                        {(migState() as Extract<MigState, {kind: "idle"}>).pendingCount} migration{
                            (migState() as Extract<MigState, {kind: "idle"}>).pendingCount !== 1 ? "s" : ""
                        } pending
                    </span>
                    <Button
                        density="compact"
                        tone="accent"
                        class="maintenance-btn maintenance-btn--primary"
                        onClick={handleRunMigrations}
                    >
                        Run
                    </Button>
                </div>
            </Show>

            <Show when={migState().kind === "running" || migState().kind === "complete" || migState().kind === "failed"}>
                <div class="maintenance-migration-block">
                    <Show when={migState().kind === "running"}>
                        <div class="maintenance-stage-header">
                            <span class="maintenance-icon maintenance-icon--working maintenance-pulse">●</span>
                            <span>Migrations running…</span>
                        </div>
                    </Show>
                    <Show when={migState().kind === "complete"}>
                        <div class="maintenance-stage-header">
                            <span class="maintenance-icon maintenance-icon--ok">✓</span>
                            <span>
                                Migrations complete
                                {" — "}
                                {(migState() as Extract<MigState, {kind: "complete"}>).applied} applied
                            </span>
                        </div>
                    </Show>
                    <Show when={migState().kind === "failed"}>
                        <div class="maintenance-stage-header">
                            <span class="maintenance-icon maintenance-icon--error">✗</span>
                            <span class="maintenance-row-text--error">
                                Migration failed
                            </span>
                        </div>
                        <div class="maintenance-error-detail">
                            {(migState() as Extract<MigState, {kind: "failed"}>).error}
                        </div>
                    </Show>
                    <For each={(migState() as Extract<MigState, {kind: "running" | "complete" | "failed"}>).steps ?? []}>
                        {(step) => (
                            <div class="maintenance-sub-row">
                                <span class={`maintenance-icon ${step.status === "done" ? "maintenance-icon--ok" : "maintenance-icon--working maintenance-pulse"}`}>
                                    {step.status === "done" ? "✓" : "·"}
                                </span>
                                <span class="maintenance-sub-id">{step.id}</span>
                                <span class="maintenance-sub-label">{step.label}</span>
                                <span class="maintenance-timer">
                                    {step.durationMs != null
                                        ? fmtDuration(step.durationMs)
                                        : elapsedSecs(step.startedAtMs, nowMs())}
                                </span>
                            </div>
                        )}
                    </For>
                    <Show when={migState().kind === "failed"}>
                        <Button
                            density="compact"
                            tone="accent"
                            class="maintenance-btn maintenance-btn--primary maintenance-retry-btn"
                            onClick={handleRunMigrations}
                        >
                            Retry
                        </Button>
                    </Show>
                </div>
            </Show>

            {/* ── Summary rows (shown when idle / after-complete) ─────────── */}
            <Show when={migState().kind === "idle"}>
                <div class="maintenance-summary-row">
                    <Show
                        when={(migState() as Extract<MigState, {kind: "idle"}>).pendingCount === 0}
                        fallback={
                            <>
                                <span class="maintenance-icon maintenance-icon--warn">⚠</span>
                                <span class="maintenance-row-label">Migrations</span>
                                <span class="maintenance-row-text--warn">
                                    {(migState() as Extract<MigState, {kind: "idle"}>).pendingCount} pending
                                </span>
                            </>
                        }
                    >
                        <span class="maintenance-icon maintenance-icon--ok">✓</span>
                        <span class="maintenance-row-label">Migrations</span>
                        <span class="maintenance-row-text--dim">up to date</span>
                    </Show>
                </div>
            </Show>

            {/* ── Saga vacuum row ────────────────────────────────────────── */}
            <div class="maintenance-summary-row">
                <Show when={vacState().kind === "idle"}>
                    <span class="maintenance-icon maintenance-icon--neutral">○</span>
                    <span class="maintenance-row-label">Vacuum</span>
                    <span class="maintenance-row-text--dim">
                        {(vacState() as Extract<VacState, {kind: "idle"}>).lastRunMs
                            ? `last run: ${fmtDate((vacState() as Extract<VacState, {kind: "idle"}>).lastRunMs!)}`
                            : "not run"}
                    </span>
                    <Button
                        density="compact"
                        class="maintenance-btn"
                        onClick={handleRunVacuum}
                    >
                        Run
                    </Button>
                </Show>
                <Show when={vacState().kind === "running"}>
                    <span class="maintenance-icon maintenance-icon--working maintenance-pulse">·</span>
                    <span class="maintenance-row-label">Vacuum</span>
                    <span class="maintenance-row-text--dim">running…</span>
                </Show>
                <Show when={vacState().kind === "done"}>
                    <span class="maintenance-icon maintenance-icon--ok">✓</span>
                    <span class="maintenance-row-label">Vacuum</span>
                    <span class="maintenance-row-text--dim">
                        {(vacState() as Extract<VacState, {kind: "done"}>).rowsDeleted === 0
                            ? "nothing to clean"
                            : `${(vacState() as Extract<VacState, {kind: "done"}>).rowsDeleted} rows removed`}
                    </span>
                    <Button density="compact" class="maintenance-btn" onClick={handleRunVacuum}>Run</Button>
                </Show>
            </div>
        </>
    );
};

MaintenanceSection.displayName = "MaintenanceSection";

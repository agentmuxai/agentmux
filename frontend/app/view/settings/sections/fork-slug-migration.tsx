// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Outdated agent ids: forks an older build named by chaining every
// generation (loap-loap-2), the id each would get today (loap-2), and
// "Update them", which runs the fork slug migration (srv
// `backend/slug_migration`, docs/specs/SPEC_FORK_SLUG_MIGRATION_2026_10_22.md
// §4 Phase 2). The plan is shown first; panes still running an affected agent
// are stopped through the usual 30 s override before anything changes, and a
// finished update can be undone from its journal. An agent whose new id is
// taken is listed with the reason and the next free id; "Use <id>" (or an id of
// the user's own) gives it that one, as the spec asks: never picked silently.

import { createSignal, For, onCleanup, onMount, Show, type Accessor, type JSX, type Setter } from "solid-js";

import { ConfirmModal } from "@/app/element/confirm-modal";
import { Button, TextInput } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { ForkSlugBlocker } from "@/types/rpc/ForkSlugBlocker";
import type { ForkSlugCandidate } from "@/types/rpc/ForkSlugCandidate";
import type { ForkSlugChoice } from "@/types/rpc/ForkSlugChoice";
import type { ForkSlugMigrationMode } from "@/types/rpc/ForkSlugMigrationMode";
import type { ForkSlugPlan } from "@/types/rpc/ForkSlugPlan";
import type { ForkSlugReport } from "@/types/rpc/ForkSlugReport";
import type { MigrateForkSlugsResult } from "@/types/rpc/MigrateForkSlugsResult";
import { settingEntry, type SettingsIndexEntry } from "../settings-model";

export const FORK_SLUG_SETTING = settingEntry({
    id: "agents.outdated_ids",
    labelKey: "settings.agents.outdatedIds.label",
    descriptionKey: "settings.agents.outdatedIds.description",
    section: "agents",
    keywords: ["fork", "slug", "agent id", "id", "rename", "migrate", "outdated", "chain"],
});

type Phase = "loading" | "checking" | "idle" | "stopping" | "applying" | "undoing";

const errorText = (e: unknown): string => (e instanceof Error ? e.message : String(e));

const sleep = (ms: number): Promise<void> => new Promise((resolve) => setTimeout(resolve, ms));

/** The candidates an update would migrate now: those whose new id is free. */
export function readyOf(plan: ForkSlugPlan | null | undefined): ForkSlugCandidate[] {
    return plan?.candidates.filter((c) => !c.refused) ?? [];
}

/** What blocks a run that stopping the affected panes can't clear: an
 *  older AgentMux running, another update under way, an unreadable
 *  definition. While any is there, no pane is stopped. */
export function unclearable(plan: ForkSlugPlan | null | undefined): ForkSlugBlocker[] {
    return plan?.blockers.filter((b) => !b.clearable_by_stopping) ?? [];
}

/** The card's status line. */
export function planLine(plan: ForkSlugPlan): string {
    if (plan.last_run && !plan.last_run.finished) {
        return plan.last_run.failed
            ? t("settings.forkSlugs.stoppedWhy", { failed: plan.last_run.failed })
            : t("settings.forkSlugs.stopped");
    }
    const n = plan.candidates.length;
    if (n === 0) return t("settings.forkSlugs.allCurrent");
    const base = t("settings.forkSlugs.outdated", { count: n });
    const refused = plan.candidates.filter((c) => c.refused).length;
    if (refused === 0) return base;
    if (refused === n) return `${base} ${t("settings.forkSlugs.noneUpdatable", { count: n })}`;
    return `${base} ${t("settings.forkSlugs.someRefused", { n: refused })}`;
}

/** A plan this old is shown again, when the card comes back into view. */
export const RECHECK_AFTER_MS = 2000;

/** Whether coming back into view (the Settings tab shown again, the window focused) should read the
 *  plan again: only when nothing is under way, and the plan on screen isn't from the last moment. The
 *  tab stays mounted while hidden, so without this it shows what was true when it was first opened. */
export function shouldRecheck(phase: Phase, loadedAt: number, now: number): boolean {
    return phase === "idle" && now - loadedAt >= RECHECK_AFTER_MS;
}

/** `load`, which also runs again when the user comes back to the card (the window focused, the tab
 *  visible, the card scrolled into view): the plan changes under it (an agent stopped, a pane closed
 *  elsewhere) and the user acts on what it says. */
function reloadOnReturn<T>(phase: () => Phase, cardId: string, load: () => Promise<T | null>): () => Promise<T | null> {
    let loadedAt = 0;
    let reading = 0;
    // The caller gets `load`'s own promise: a wrapper that awaited it would answer a tick later. The
    // plan's age counts from when the read started, and a read is never started while another is under way.
    const stamped = (): Promise<T | null> => {
        loadedAt = Date.now();
        reading++;
        const loading = load();
        void loading.finally(() => reading--);
        return loading;
    };
    const recheck = () => {
        if (reading === 0 && shouldRecheck(phase(), loadedAt, Date.now())) void stamped();
    };
    onMount(() => {
        const onVisible = () => {
            if (document.visibilityState === "visible") recheck();
        };
        window.addEventListener("focus", recheck);
        document.addEventListener("visibilitychange", onVisible);
        const card = document.getElementById(cardId);
        const seen =
            typeof IntersectionObserver === "undefined" || !card
                ? undefined
                : new IntersectionObserver((entries) => {
                      if (entries.some((e) => e.isIntersecting)) recheck();
                  });
        if (card) seen?.observe(card);
        onCleanup(() => {
            window.removeEventListener("focus", recheck);
            document.removeEventListener("visibilitychange", onVisible);
            seen?.disconnect();
        });
    });
    return stamped;
}

function call(
    mode: ForkSlugMigrationMode,
    journal?: string,
    choices?: ForkSlugChoice[]
): Promise<MigrateForkSlugsResult> {
    const data = { mode, ...(journal ? { journal } : {}), ...(choices && choices.length > 0 ? { choices } : {}) };
    return RpcApi.AgentsMigrateForkSlugsCommand(TabRpcClient, data);
}

/** What the card's steps share: the signals, and the read of the plan (which also runs again on return). */
interface MigrationState {
    plan: Accessor<ForkSlugPlan | null>;
    setPlan: Setter<ForkSlugPlan | null>;
    report: Accessor<ForkSlugReport | null>;
    setReport: Setter<ForkSlugReport | null>;
    error: Accessor<string | null>;
    setError: Setter<string | null>;
    phase: Accessor<Phase>;
    setPhase: Setter<Phase>;
    confirmStop: Accessor<boolean>;
    setConfirmStop: Setter<boolean>;
    confirmUndo: Accessor<boolean>;
    setConfirmUndo: Setter<boolean>;
    showWhere: Accessor<boolean>;
    setShowWhere: Setter<boolean>;
    // The id the user picked for an agent whose own is taken; sent with every
    // call until the update that uses it is done or given up.
    chosen: Accessor<ForkSlugChoice[]>;
    setChosen: Setter<ForkSlugChoice[]>;
    customFor: Accessor<string | null>;
    setCustomFor: Setter<string | null>;
    customId: Accessor<string>;
    setCustomId: Setter<string>;
    load: () => Promise<ForkSlugPlan | null>;
}

function createMigrationState(): MigrationState {
    const [plan, setPlan] = createSignal<ForkSlugPlan | null>(null);
    const [report, setReport] = createSignal<ForkSlugReport | null>(null);
    const [error, setError] = createSignal<string | null>(null);
    const [phase, setPhase] = createSignal<Phase>("loading");
    const [confirmStop, setConfirmStop] = createSignal(false);
    const [confirmUndo, setConfirmUndo] = createSignal(false);
    const [showWhere, setShowWhere] = createSignal(false);
    const [chosen, setChosen] = createSignal<ForkSlugChoice[]>([]);
    const [customFor, setCustomFor] = createSignal<string | null>(null);
    const [customId, setCustomId] = createSignal("");
    const s: MigrationState = {
        plan,
        setPlan,
        report,
        setReport,
        error,
        setError,
        phase,
        setPhase,
        confirmStop,
        setConfirmStop,
        confirmUndo,
        setConfirmUndo,
        showWhere,
        setShowWhere,
        chosen,
        setChosen,
        customFor,
        setCustomFor,
        customId,
        setCustomId,
        load: undefined as unknown as MigrationState["load"], // set just below: it needs the state
    };
    s.load = reloadOnReturn(phase, `setting-${FORK_SLUG_SETTING.id}`, () => readPlan(s));
    return s;
}

async function readPlan(s: MigrationState): Promise<ForkSlugPlan | null> {
    try {
        const p = (await call("plan", undefined, s.chosen())).plan ?? null;
        s.setPlan(p);
        return p;
    } catch (e) {
        s.setError(t("settings.forkSlugs.checkFailed", { error: errorText(e) }));
        return null;
    }
}

const ready = (s: MigrationState) => readyOf(s.plan());
const running = (s: MigrationState) => ready(s).flatMap((c) => c.running.map((r) => ({ name: c.name, ...r })));
const blockers = (s: MigrationState) => s.plan()?.blockers ?? [];
// An srv from before the report sends none.
const duplicates = (s: MigrationState) => s.plan()?.duplicates ?? [];
const busy = (s: MigrationState) => s.phase() !== "idle";
// Running panes are the one blocker the card clears itself, by stopping
// them; anything else (an older AgentMux running, another update under
// way) is the user's to clear, and stopping panes for it would be for
// nothing.
const canStart = (s: MigrationState) => !busy(s) && ready(s).length > 0 && unclearable(s.plan()).length === 0;

async function apply(s: MigrationState) {
    s.setPhase("applying");
    s.setError(null);
    try {
        const r = await call("apply", undefined, s.chosen());
        s.setReport(r.report ?? null);
        if (r.report?.failed) s.setError(t("settings.forkSlugs.updateStopped", { failed: r.report.failed }));
    } catch (e) {
        s.setError(t("settings.forkSlugs.updateFailed", { error: errorText(e) }));
    }
    s.setChosen([]);
    await s.load();
    s.setPhase("idle");
}

async function stopThenApply(s: MigrationState) {
    s.setConfirmStop(false);
    s.setPhase("stopping");
    s.setError(null);
    // Read again before stopping anything: something the stop can't
    // clear may have appeared since the plan was shown.
    const fresh = await s.load();
    if (!fresh || unclearable(fresh).length > 0 || readyOf(fresh).length === 0) {
        s.setPhase("idle");
        return;
    }
    try {
        const r = await call("stop", undefined, s.chosen());
        if (r.wait_ms > 0) await sleep(r.wait_ms + 1000);
    } catch (e) {
        s.setError(t("settings.forkSlugs.stopFailed", { error: errorText(e) }));
        s.setPhase("idle");
        return;
    }
    const p = await s.load();
    if (p && p.blockers.length === 0 && readyOf(p).length > 0) {
        await apply(s);
    } else {
        s.setPhase("idle");
    }
}

function start(s: MigrationState) {
    if (running(s).length > 0) s.setConfirmStop(true);
    else void apply(s);
}

// An agent can be given another id only once the ones the rule can name are
// done: the update then has nothing else to do, and a pick changes nothing
// but that agent.
// Not while another pick is being checked or waits on the stop dialog: one
// pick at a time, so what is applied is what was checked.
const canPick = (s: MigrationState) =>
    !busy(s) && !s.confirmStop() && ready(s).length === 0 && unclearable(s.plan()).length === 0;

/** Give `c` the id `slug` instead of its own (taken) one: check it, then update as "Update them" does. */
async function useId(s: MigrationState, c: ForkSlugCandidate, slug: string) {
    const id = slug.trim();
    if (!id || !canPick(s)) return;
    s.setPhase("checking");
    s.setError(null);
    s.setChosen([{ uid: c.uid, new_slug: id }]);
    const p = await s.load();
    const mine = p?.candidates.find((x) => x.uid === c.uid);
    if (!mine || mine.refused) {
        // Taken after all, or not an id: say so, and show the plan as it was.
        if (mine?.refused) s.setError(t("settings.forkSlugs.idRefused", { id, why: mine.refused }));
        s.setChosen([]);
        await s.load();
        s.setPhase("idle");
        return;
    }
    s.setCustomFor(null);
    s.setCustomId("");
    s.setPhase("idle");
    start(s);
}

function cancelStop(s: MigrationState) {
    s.setConfirmStop(false);
    if (s.chosen().length > 0) {
        s.setChosen([]);
        void s.load();
    }
}

// The run Undo reverses: this session's, or the latest one on disk, so
// Undo outlives the session that ran it.
const undoJournal = (s: MigrationState) => s.report()?.journal || s.plan()?.last_run?.journal;
const unfinished = (s: MigrationState) =>
    s.plan()?.last_run && !s.plan()!.last_run!.finished ? s.plan()!.last_run! : null;

async function undo(s: MigrationState) {
    const journal = undoJournal(s);
    s.setConfirmUndo(false);
    if (!journal) return;
    s.setPhase("undoing");
    s.setError(null);
    try {
        await call("rollback", journal);
        s.setReport(null);
    } catch (e) {
        s.setError(t("settings.forkSlugs.undoFailed", { error: errorText(e) }));
    }
    await s.load();
    s.setPhase("idle");
}

const statusLine = (s: MigrationState) => {
    switch (s.phase()) {
        case "loading":
        case "checking":
            return t("settings.forkSlugs.checking");
        case "stopping":
            return t("settings.forkSlugs.stopping");
        case "applying":
            return t("settings.forkSlugs.updating");
        case "undoing":
            return t("settings.forkSlugs.undoing");
    }
    const p = s.plan();
    return p ? planLine(p) : "";
};

export function ForkSlugMigration(): JSX.Element {
    const s = createMigrationState();
    onMount(async () => {
        await s.load();
        s.setPhase("idle");
    });

    return (
        <div id={`setting-${FORK_SLUG_SETTING.id}`} class="setting-row">
            <div class="setting-fork-slugs-description">{FORK_SLUG_SETTING.description}</div>
            <StatusRow s={s} />
            <Show when={s.error()}>
                {(e) => (
                    <div class="settings-config-error" role="alert">
                        {e()}
                    </div>
                )}
            </Show>
            <CandidateList s={s} />
            <Show when={blockers(s).length > 0 && !busy(s)}>
                <ul class="setting-fork-slugs-blockers" aria-label={t("settings.forkSlugs.blockersAria")}>
                    <For each={blockers(s)}>{(b) => <li>{b.message}</li>}</For>
                </ul>
            </Show>
            <DuplicateList s={s} />
            <Show when={s.report()}>{(r) => <ReportView r={r()} />}</Show>
            <Show when={undoJournal(s)}>
                <div class="setting-fork-slugs-actions">
                    <Button tone="quiet" density="compact" disabled={busy(s)} onClick={() => s.setConfirmUndo(true)}>
                        {t("settings.forkSlugs.undo")}
                    </Button>
                </div>
            </Show>
            <StopModal s={s} />
            <UndoModal s={s} />
        </div>
    );
}

const StatusRow = (p: { s: MigrationState }): JSX.Element => {
    const s = p.s;
    return (
        <div class="setting-fork-slugs">
            <span class="setting-fork-slugs-line" role="status">
                {statusLine(s)}
            </span>
            <Show when={unfinished(s)}>
                <Button tone="accent" busy={s.phase() === "applying"} disabled={busy(s)} onClick={() => void apply(s)}>
                    {t("settings.forkSlugs.finish")}
                </Button>
            </Show>
            <Show when={!unfinished(s) && (s.plan()?.candidates.length ?? 0) > 0}>
                <Button
                    tone="accent"
                    busy={s.phase() === "applying" || s.phase() === "stopping"}
                    disabled={!canStart(s)}
                    onClick={() => start(s)}
                >
                    {t("settings.forkSlugs.updateThem")}
                </Button>
            </Show>
        </div>
    );
};

const CandidateList = (p: { s: MigrationState }): JSX.Element => {
    const s = p.s;
    return (
        <Show when={(s.plan()?.candidates.length ?? 0) > 0}>
            <ul class="setting-fork-slugs-list" aria-label={t("settings.forkSlugs.listAria")}>
                <For each={s.plan()!.candidates}>{(c) => <CandidateItem s={s} c={c} />}</For>
            </ul>
            <div class="setting-fork-slugs-actions">
                <Button tone="quiet" density="compact" onClick={() => s.setShowWhere(!s.showWhere())}>
                    {s.showWhere() ? t("settings.forkSlugs.hideWhere") : t("settings.forkSlugs.showWhere")}
                </Button>
            </div>
        </Show>
    );
};

const CandidateItem = (p: { s: MigrationState; c: ForkSlugCandidate }): JSX.Element => {
    const { s, c } = p;
    return (
        <li>
            <span class="setting-fork-slugs-name">{c.name}</span> <span class="mono">{c.old_slug}</span> →{" "}
            <span class="mono">{c.new_slug}</span>
            <Show when={c.refused}>{(why) => <RefusedNote s={s} c={c} why={why()} />}</Show>
            <Show when={!c.refused && c.running.length > 0 && unclearable(s.plan()).length === 0}>
                <div class="setting-fork-slugs-note">{t("settings.forkSlugs.running")}</div>
            </Show>
            <Show when={s.showWhere()}>
                <ul class="setting-fork-slugs-where">
                    <For each={c.locations}>
                        {(l) => (
                            <li>
                                <span class="mono">{l.key}</span> {l.value}{" "}
                                <span class="setting-fork-slugs-place">{l.place}</span>
                            </li>
                        )}
                    </For>
                </ul>
            </Show>
        </li>
    );
};

const RefusedNote = (p: { s: MigrationState; c: ForkSlugCandidate; why: string }): JSX.Element => {
    const { s, c } = p;
    return (
        <div class="setting-fork-slugs-note">
            {t("settings.forkSlugs.notNow", { why: p.why })}
            <Show when={c.suggestion}>{(sug) => <> {t("settings.forkSlugs.nextFree", { id: sug() })}</>}</Show>
            <Show when={ready(s).length > 0}>
                <div>{t("settings.forkSlugs.updateOthersFirst")}</div>
            </Show>
            <div class="setting-fork-slugs-actions">
                <Show when={c.suggestion}>
                    {(sug) => (
                        <Button
                            tone="accent"
                            density="compact"
                            disabled={!canPick(s)}
                            onClick={() => void useId(s, c, sug())}
                        >
                            {t("settings.forkSlugs.use", { id: sug() })}
                        </Button>
                    )}
                </Show>
                <Button
                    tone="quiet"
                    density="compact"
                    disabled={!canPick(s)}
                    onClick={() => s.setCustomFor(s.customFor() === c.uid ? null : c.uid)}
                >
                    {t("settings.forkSlugs.chooseAnother")}
                </Button>
            </div>
            <Show when={s.customFor() === c.uid}>
                <CustomIdRow s={s} c={c} />
            </Show>
        </div>
    );
};

const CustomIdRow = (p: { s: MigrationState; c: ForkSlugCandidate }): JSX.Element => {
    const { s, c } = p;
    return (
        <div class="setting-fork-slugs-actions">
            <TextInput
                density="compact"
                aria-label={t("settings.forkSlugs.newIdFor", { name: c.name })}
                placeholder={t("settings.forkSlugs.newIdPlaceholder")}
                value={s.customId()}
                onInput={(e) => s.setCustomId(e.currentTarget.value)}
                onKeyDown={(e) => {
                    if (e.key === "Enter") void useId(s, c, s.customId());
                }}
            />
            <Button
                tone="accent"
                density="compact"
                disabled={!canPick(s) || s.customId().trim() === ""}
                onClick={() => void useId(s, c, s.customId())}
            >
                {t("settings.forkSlugs.useThis")}
            </Button>
        </div>
    );
};

const DuplicateList = (p: { s: MigrationState }): JSX.Element => {
    const s = p.s;
    return (
        <Show when={duplicates(s).length > 0}>
            <div class="setting-fork-slugs-note">
                {t("settings.forkSlugs.shareNote")}
            </div>
            <ul class="setting-fork-slugs-list" aria-label={t("settings.forkSlugs.shareAria")}>
                <For each={duplicates(s)}>
                    {(d) => (
                        <li>
                            <span class="mono">{d.slug}</span>
                            <ul class="setting-fork-slugs-where">
                                <For each={d.holders}>
                                    {(h, i) => (
                                        <li>
                                            <span class="setting-fork-slugs-name">{h.name}</span>{" "}
                                            <span class="setting-fork-slugs-place">{h.place}</span>
                                            {i() === 0 ? (
                                                <> {t("settings.forkSlugs.keeps")}</>
                                            ) : h.proposed ? (
                                                <>
                                                    {" "}
                                                    {t("settings.forkSlugs.couldRename")} <span class="mono">{h.proposed}</span>
                                                </>
                                            ) : (
                                                <> {t("settings.forkSlugs.needsNew")}</>
                                            )}
                                        </li>
                                    )}
                                </For>
                            </ul>
                        </li>
                    )}
                </For>
            </ul>
        </Show>
    );
};

const ReportView = (p: { r: ForkSlugReport }): JSX.Element => (
    <div class="setting-fork-slugs-report" aria-label={t("settings.forkSlugs.reportAria")}>
        <For each={p.r.migrated}>
            {(m) => (
                <div>
                    {t("settings.forkSlugs.updated", { name: m.name })} <span class="mono">{m.old_slug}</span> → <span class="mono">{m.new_slug}</span>
                </div>
            )}
        </For>
        <Show when={p.r.pr_searches.length > 0}>
            <div class="setting-fork-slugs-note">{t("settings.forkSlugs.prNote")}</div>
            <pre class="setting-fork-slugs-commands">{p.r.pr_searches.join("\n")}</pre>
        </Show>
        <For each={p.r.notes}>{(n) => <div class="setting-fork-slugs-note">{n}</div>}</For>
    </div>
);

const StopModal = (p: { s: MigrationState }): JSX.Element => {
    const s = p.s;
    return (
        <ConfirmModal
            open={s.confirmStop()}
            destructive
            scope="pane"
            title={t("settings.forkSlugs.stopTitle", { count: running(s).length })}
            description={t("settings.forkSlugs.stopDescription")}
            confirmLabel={t("settings.forkSlugs.stopConfirm")}
            onConfirm={() => stopThenApply(s)}
            onCancel={() => cancelStop(s)}
        >
            <ul class="setting-fork-slugs-list">
                <For each={running(s)}>
                    {(r) => (
                        <li>
                            {r.name}
                            {r.channel ? ` (${r.channel})` : ""}
                        </li>
                    )}
                </For>
            </ul>
        </ConfirmModal>
    );
};

const UndoModal = (p: { s: MigrationState }): JSX.Element => {
    const s = p.s;
    return (
        <ConfirmModal
            open={s.confirmUndo()}
            destructive
            scope="pane"
            title={t("settings.forkSlugs.undoTitle")}
            description={t("settings.forkSlugs.undoDescription")}
            confirmLabel={t("settings.forkSlugs.undoConfirm")}
            onConfirm={() => undo(s)}
            onCancel={() => s.setConfirmUndo(false)}
        />
    );
};

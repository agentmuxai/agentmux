// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Outdated agent ids: forks an older build named by chaining every
// generation (loap-loap-2), the id each would get today (loap-2), and
// "Update them", which runs the fork slug migration (srv
// `backend/slug_migration`, docs/specs/SPEC_FORK_SLUG_MIGRATION_2026_10_22.md
// §4 Phase 2). The plan is shown first; panes still running an affected agent
// are stopped through the usual 15 s override before anything changes, and a
// finished update can be undone from its journal. An agent whose new id is
// taken is listed with the reason and the next free id; "Use <id>" (or an id of
// the user's own) gives it that one, as the spec asks: never picked silently.

import { createSignal, For, onMount, Show, type JSX } from "solid-js";

import { ConfirmModal } from "@/app/element/confirm-modal";
import { Button, TextInput } from "@/app/element/ui";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { ForkSlugBlocker } from "@/types/rpc/ForkSlugBlocker";
import type { ForkSlugCandidate } from "@/types/rpc/ForkSlugCandidate";
import type { ForkSlugChoice } from "@/types/rpc/ForkSlugChoice";
import type { ForkSlugMigrationMode } from "@/types/rpc/ForkSlugMigrationMode";
import type { ForkSlugPlan } from "@/types/rpc/ForkSlugPlan";
import type { ForkSlugReport } from "@/types/rpc/ForkSlugReport";
import type { MigrateForkSlugsResult } from "@/types/rpc/MigrateForkSlugsResult";
import type { SettingsIndexEntry } from "../settings-model";

export const FORK_SLUG_SETTING = {
    id: "agents.outdated_ids",
    label: "Outdated agent ids",
    description:
        "Forks made by older builds got an id that repeats every generation (loap-loap-2-loap-3). Updating gives each the id it would get today (loap-3). The old id still receives messages and review notices for 30 days, and is never given to another agent.",
    section: "agents",
    keywords: ["fork", "slug", "agent id", "id", "rename", "migrate", "outdated", "chain"],
} satisfies SettingsIndexEntry;

type Phase = "loading" | "idle" | "stopping" | "applying" | "undoing";

const plural = (n: number, one: string, many: string = `${one}s`): string => `${n} ${n === 1 ? one : many}`;

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
        return plan.last_run.failed ? `An update stopped before it finished: ${plan.last_run.failed}` : "An update stopped before it finished.";
    }
    const n = plan.candidates.length;
    if (n === 0) return "Every agent has an up-to-date id.";
    const base = `${plural(n, "agent")} ${n === 1 ? "has" : "have"} an outdated id.`;
    const refused = plan.candidates.filter((c) => c.refused).length;
    if (refused === 0) return base;
    if (refused === n) return `${base} ${n === 1 ? "It can't" : "None can"} be updated yet: pick another id below.`;
    return `${base} ${refused} can't be updated yet: pick another id below.`;
}

function call(mode: ForkSlugMigrationMode, journal?: string, choices?: ForkSlugChoice[]): Promise<MigrateForkSlugsResult> {
    const data = { mode, ...(journal ? { journal } : {}), ...(choices && choices.length > 0 ? { choices } : {}) };
    return RpcApi.AgentsMigrateForkSlugsCommand(TabRpcClient, data);
}

export function ForkSlugMigration(): JSX.Element {
    const [plan, setPlan] = createSignal<ForkSlugPlan | null>(null);
    const [report, setReport] = createSignal<ForkSlugReport | null>(null);
    const [error, setError] = createSignal<string | null>(null);
    const [phase, setPhase] = createSignal<Phase>("loading");
    const [confirmStop, setConfirmStop] = createSignal(false);
    const [confirmUndo, setConfirmUndo] = createSignal(false);
    const [showWhere, setShowWhere] = createSignal(false);
    // The id the user picked for an agent whose own is taken; sent with every
    // call until the update that uses it is done or given up.
    const [chosen, setChosen] = createSignal<ForkSlugChoice[]>([]);
    const [customFor, setCustomFor] = createSignal<string | null>(null);
    const [customId, setCustomId] = createSignal("");

    const load = async (): Promise<ForkSlugPlan | null> => {
        try {
            const p = (await call("plan", undefined, chosen())).plan ?? null;
            setPlan(p);
            return p;
        } catch (e) {
            setError(`Couldn't check the agent ids: ${errorText(e)}`);
            return null;
        }
    };
    onMount(async () => {
        await load();
        setPhase("idle");
    });

    const ready = () => readyOf(plan());
    const running = () => ready().flatMap((c) => c.running.map((r) => ({ name: c.name, ...r })));
    const blockers = () => plan()?.blockers ?? [];
    // An srv from before the report sends none.
    const duplicates = () => plan()?.duplicates ?? [];
    const busy = () => phase() !== "idle";
    // Running panes are the one blocker the card clears itself, by stopping
    // them; anything else (an older AgentMux running, another update under
    // way) is the user's to clear, and stopping panes for it would be for
    // nothing.
    const canStart = () => !busy() && ready().length > 0 && unclearable(plan()).length === 0;

    const apply = async () => {
        setPhase("applying");
        setError(null);
        try {
            const r = await call("apply", undefined, chosen());
            setReport(r.report ?? null);
            if (r.report?.failed) setError(`The update stopped: ${r.report.failed}`);
        } catch (e) {
            setError(`Couldn't update: ${errorText(e)}`);
        }
        setChosen([]);
        await load();
        setPhase("idle");
    };

    const stopThenApply = async () => {
        setConfirmStop(false);
        setPhase("stopping");
        setError(null);
        // Read again before stopping anything: something the stop can't
        // clear may have appeared since the plan was shown.
        const fresh = await load();
        if (!fresh || unclearable(fresh).length > 0 || readyOf(fresh).length === 0) {
            setPhase("idle");
            return;
        }
        try {
            const r = await call("stop", undefined, chosen());
            if (r.wait_ms > 0) await sleep(r.wait_ms + 1000);
        } catch (e) {
            setError(`Couldn't stop the panes: ${errorText(e)}`);
            setPhase("idle");
            return;
        }
        const p = await load();
        if (p && p.blockers.length === 0 && readyOf(p).length > 0) {
            await apply();
        } else {
            setPhase("idle");
        }
    };

    const start = () => {
        if (running().length > 0) setConfirmStop(true);
        else void apply();
    };

    // An agent can be given another id only once the ones the rule can name are
    // done: the update then has nothing else to do, and a pick changes nothing
    // but that agent.
    const canPick = () => !busy() && ready().length === 0 && unclearable(plan()).length === 0;

    /** Give `c` the id `slug` instead of its own (taken) one: check it, then update as "Update them" does. */
    const useId = async (c: ForkSlugCandidate, slug: string) => {
        const id = slug.trim();
        if (!id || !canPick()) return;
        setError(null);
        setChosen([{ uid: c.uid, new_slug: id }]);
        const p = await load();
        const mine = p?.candidates.find((x) => x.uid === c.uid);
        if (!mine || mine.refused) {
            // Taken after all, or not an id: say so, and show the plan as it was.
            if (mine?.refused) setError(`${id} can't be used: ${mine.refused}`);
            setChosen([]);
            await load();
            return;
        }
        setCustomFor(null);
        setCustomId("");
        start();
    };

    const cancelStop = () => {
        setConfirmStop(false);
        if (chosen().length > 0) {
            setChosen([]);
            void load();
        }
    };

    // The run Undo reverses: this session's, or the latest one on disk, so
    // Undo outlives the session that ran it.
    const undoJournal = () => report()?.journal || plan()?.last_run?.journal;
    const unfinished = () => (plan()?.last_run && !plan()!.last_run!.finished ? plan()!.last_run! : null);

    const undo = async () => {
        const journal = undoJournal();
        setConfirmUndo(false);
        if (!journal) return;
        setPhase("undoing");
        setError(null);
        try {
            await call("rollback", journal);
            setReport(null);
        } catch (e) {
            setError(`Couldn't undo the update: ${errorText(e)}`);
        }
        await load();
        setPhase("idle");
    };

    const line = () => {
        switch (phase()) {
            case "loading":
                return "Checking agent ids…";
            case "stopping":
                return "Stopping the panes. Each can still be kept running from its own pane for 15 s.";
            case "applying":
                return "Updating…";
            case "undoing":
                return "Undoing the update…";
        }
        const p = plan();
        return p ? planLine(p) : "";
    };

    return (
        <div id={`setting-${FORK_SLUG_SETTING.id}`} class="setting-row">
            <div class="setting-fork-slugs-description">{FORK_SLUG_SETTING.description}</div>
            <div class="setting-fork-slugs">
                <span class="setting-fork-slugs-line" role="status">
                    {line()}
                </span>
                <Show when={unfinished()}>
                    <Button tone="accent" busy={phase() === "applying"} disabled={busy()} onClick={() => void apply()}>
                        Finish the update
                    </Button>
                </Show>
                <Show when={!unfinished() && (plan()?.candidates.length ?? 0) > 0}>
                    <Button tone="accent" busy={phase() === "applying" || phase() === "stopping"} disabled={!canStart()} onClick={start}>
                        Update them
                    </Button>
                </Show>
            </div>
            <Show when={error()}>
                {(e) => (
                    <div class="settings-config-error" role="alert">
                        {e()}
                    </div>
                )}
            </Show>
            <Show when={(plan()?.candidates.length ?? 0) > 0}>
                <ul class="setting-fork-slugs-list" aria-label="Agents with an outdated id">
                    <For each={plan()!.candidates}>
                        {(c) => (
                            <li>
                                <span class="setting-fork-slugs-name">{c.name}</span>{" "}
                                <span class="mono">{c.old_slug}</span> → <span class="mono">{c.new_slug}</span>
                                <Show when={c.refused}>
                                    {(why) => (
                                        <div class="setting-fork-slugs-note">
                                            Not now: {why()}
                                            <Show when={c.suggestion}>{(s) => <> (the next free id is {s()})</>}</Show>
                                            <Show when={ready().length > 0}>
                                                <div>Update the other agents first; then this one can take another id.</div>
                                            </Show>
                                            <div class="setting-fork-slugs-actions">
                                                <Show when={c.suggestion}>
                                                    {(s) => (
                                                        <Button tone="accent" density="compact" disabled={!canPick()} onClick={() => void useId(c, s())}>
                                                            Use {s()}
                                                        </Button>
                                                    )}
                                                </Show>
                                                <Button
                                                    tone="quiet"
                                                    density="compact"
                                                    disabled={!canPick()}
                                                    onClick={() => setCustomFor(customFor() === c.uid ? null : c.uid)}
                                                >
                                                    Choose another id
                                                </Button>
                                            </div>
                                            <Show when={customFor() === c.uid}>
                                                <div class="setting-fork-slugs-actions">
                                                    <TextInput
                                                        density="compact"
                                                        aria-label={`New id for ${c.name}`}
                                                        placeholder="a-new-id"
                                                        value={customId()}
                                                        onInput={(e) => setCustomId(e.currentTarget.value)}
                                                        onKeyDown={(e) => {
                                                            if (e.key === "Enter") void useId(c, customId());
                                                        }}
                                                    />
                                                    <Button
                                                        tone="accent"
                                                        density="compact"
                                                        disabled={!canPick() || customId().trim() === ""}
                                                        onClick={() => void useId(c, customId())}
                                                    >
                                                        Use this id
                                                    </Button>
                                                </div>
                                            </Show>
                                        </div>
                                    )}
                                </Show>
                                <Show when={!c.refused && c.running.length > 0 && unclearable(plan()).length === 0}>
                                    <div class="setting-fork-slugs-note">Running: it is stopped first.</div>
                                </Show>
                                <Show when={showWhere()}>
                                    <ul class="setting-fork-slugs-where">
                                        <For each={c.locations}>
                                            {(l) => (
                                                <li>
                                                    <span class="mono">{l.key}</span> {l.value} <span class="setting-fork-slugs-place">{l.place}</span>
                                                </li>
                                            )}
                                        </For>
                                    </ul>
                                </Show>
                            </li>
                        )}
                    </For>
                </ul>
                <div class="setting-fork-slugs-actions">
                    <Button tone="quiet" density="compact" onClick={() => setShowWhere(!showWhere())}>
                        {showWhere() ? "Hide where the old ids are stored" : "Show where the old ids are stored"}
                    </Button>
                </div>
            </Show>
            <Show when={blockers().length > 0 && !busy()}>
                <ul class="setting-fork-slugs-blockers" aria-label="What has to change first">
                    <For each={blockers()}>{(b) => <li>{b.message}</li>}</For>
                </ul>
            </Show>
            <Show when={duplicates().length > 0}>
                <div class="setting-fork-slugs-note">
                    These agents share an id, as older builds could leave them. An update doesn't change them; the
                    oldest keeps the id.
                </div>
                <ul class="setting-fork-slugs-list" aria-label="Agents that share an id">
                    <For each={duplicates()}>
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
                                                    " keeps it"
                                                ) : h.proposed ? (
                                                    <>
                                                        {" "}
                                                        could be renamed to <span class="mono">{h.proposed}</span>
                                                    </>
                                                ) : (
                                                    " needs a new id"
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
            <Show when={report()}>
                {(r) => (
                    <div class="setting-fork-slugs-report" aria-label="Update report">
                        <For each={r().migrated}>
                            {(m) => (
                                <div>
                                    Updated {m.name}: <span class="mono">{m.old_slug}</span> → <span class="mono">{m.new_slug}</span>
                                </div>
                            )}
                        </For>
                        <Show when={r().pr_searches.length > 0}>
                            <div class="setting-fork-slugs-note">Open PRs may still name an old id. Find them with:</div>
                            <pre class="setting-fork-slugs-commands">{r().pr_searches.join("\n")}</pre>
                        </Show>
                        <For each={r().notes}>{(n) => <div class="setting-fork-slugs-note">{n}</div>}</For>
                    </div>
                )}
            </Show>
            <Show when={undoJournal()}>
                <div class="setting-fork-slugs-actions">
                    <Button tone="quiet" density="compact" disabled={busy()} onClick={() => setConfirmUndo(true)}>
                        Undo the update
                    </Button>
                </div>
            </Show>
            <ConfirmModal
                open={confirmStop()}
                destructive
                title={`Stop ${plural(running().length, "pane")}?`}
                description="These panes run agents being updated. Each one shows a Keep running button for 15 seconds before it stops; the update starts once they have."
                confirmLabel="Stop and update"
                onConfirm={stopThenApply}
                onCancel={cancelStop}
            >
                <ul class="setting-fork-slugs-list">
                    <For each={running()}>{(r) => <li>{r.name}{r.channel ? ` (${r.channel})` : ""}</li>}</For>
                </ul>
            </ConfirmModal>
            <ConfirmModal
                open={confirmUndo()}
                destructive
                title="Undo the update?"
                description="Every change it made is reversed, step by step, from its journal, and the agents take their old ids at their next launch."
                confirmLabel="Undo"
                onConfirm={undo}
                onCancel={() => setConfirmUndo(false)}
            />
        </div>
    );
}

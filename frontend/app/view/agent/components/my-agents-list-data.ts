// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The My agents list's data: what it asks srv for (and again when an agent changes), the rows it keeps for the
// list's animations, what the name filter and the sort make of them, and which states it is in (loading, failed,
// empty, nothing matches). Split out of MyAgentsList.tsx (#13044 S6); the component keeps the markup.

import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { literalFirstSearch } from "@/app/util/fuzzysearch";
import type { RecentSessionRow } from "@/types/rpc/RecentSessionRow";
import { Logger } from "@/util/logger";
import { createEffect, createMemo, createResource, createSignal, onCleanup, type Accessor } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import type { AgentSortOption } from "./AgentPickerFilterBar";
import { compareRows } from "./my-agents-list-text";

/** Backend hard cap (`CommandListRecentSessionsData`, instance.rs) — the
 * limit MyAgentsList requests once a name filter is active, so filtering
 * covers the full available set instead of just the default first page. */
const SEARCH_LIMIT = 100;

/** The parts of the list's props that decide what is fetched and shown. */
export interface MyAgentsDataOptions {
    identityId?: Accessor<string | null | undefined>;
    limit?: number;
    nameFilter?: Accessor<string | null | undefined>;
    sortBy?: Accessor<AgentSortOption>;
    onFirstLoad?: () => void;
}

export function useMyAgentsData(props: MyAgentsDataOptions) {
    const filterId = createMemo(() => {
        const raw = props.identityId?.();
        if (raw === null || raw === undefined) return "";
        return String(raw).trim();
    });

    // Trimmed query, kept in its original case for display (the no-match
    // message echoes back what the user typed). `nameQuery` below is the
    // lowercased form used for matching.
    const rawQuery = createMemo(() => {
        const raw = props.nameFilter?.();
        if (raw === null || raw === undefined) return "";
        return String(raw).trim();
    });
    const nameQuery = createMemo(() => rawQuery().toLowerCase());
    const isSearching = createMemo(() => nameQuery().length > 0);

    // Compound resource key: a plain string so Solid's resource-source
    // memoization (value equality) only triggers a refetch when the
    // identity filter changes OR searching flips on/off — NOT on every
    // keystroke while already searching. An object key would defeat this
    // (a fresh object each recomputation is never `Object.is`-equal to the
    // last one), so the two parts are joined into one primitive instead of
    // passed as `{ id, searching }`.
    const resourceKey = createMemo(() => `${filterId()} ${isSearching() ? "1" : "0"}`);

    // Separate from the resource's own value: `rows()` still resolves to
    // `[]` on a failed fetch (see the catch below) so nothing here needs
    // to special-case Solid's throw-on-read error-state accessor — this
    // signal is the ONLY thing that distinguishes "backend call failed"
    // from "genuinely zero agents" for the render below. Cleared at the
    // start of every fetch so a stale error doesn't survive into a fresh
    // attempt's loading state.
    const [fetchError, setFetchError] = createSignal(false);
    // Guards the two `setFetchError(true)` calls below against a stale
    // (superseded) fetch's late resolution overwriting a NEWER fetch's
    // already-settled state — e.g. rapid identity-filter switching, or
    // clicking Retry again before the first attempt has finished, could
    // otherwise let an old failure paint the error panel over valid,
    // already-loaded data from the fetch that actually matters now
    // (reagent P1 on PR #2327's re-review). Incremented synchronously at
    // the start of each fetcher call, so invocation order is always
    // correct even though resolution order isn't.
    let fetchGeneration = 0;

    const [rows, { refetch }] = createResource<RecentSessionRow[], string>(
        // Key the resource on identity + whether a name search is active
        // (see resourceKey's own comment) so changes to either refetch.
        resourceKey,
        async () => {
            // Read the current values directly rather than parsing
            // `resourceKey`'s string — the key's only job is to give
            // Solid's resource-source memoization a primitive to dedupe
            // on (see resourceKey's own comment); by the time this
            // fetcher runs, `filterId()`/`isSearching()` already reflect
            // whatever change just triggered it.
            const id = filterId();
            const myGeneration = ++fetchGeneration;
            setFetchError(false);
            try {
                const result = await RpcApi.ListRecentSessionsCommand(TabRpcClient, {
                    limit: isSearching() ? SEARCH_LIMIT : (props.limit ?? 20),
                    // Backend treats "" as "no filter" — see
                    // CommandListRecentSessionsData docs.
                    identity_id: id,
                });
                // Zero rows AND a reported degradation means a backend data
                // source failed and we got nothing back — NOT a trustworthy
                // "you have no agents." A healthy call with genuinely zero
                // agents never populates `degraded` (session.rs's six
                // sources only degrade on their own error, never on "found
                // nothing"). Partial degradation alongside real rows (e.g.
                // identity lookups failing but the registry/defs succeeding)
                // is left alone here — those rows still render, just with
                // the account line's own fallback ("No auth" or no line).
                if (result.rows.length === 0 && result.degraded.length > 0) {
                    Logger.error("agent", "MyAgentsList: listrecentsessions reported degraded sources with zero rows", {
                        degraded: result.degraded,
                        identityId: id,
                    });
                    if (myGeneration === fetchGeneration) setFetchError(true);
                    return [];
                }
                return result.rows;
            } catch (e) {
                // Was a silent `catch { return []; }` — indistinguishable from
                // "genuinely no sessions" in the UI and left zero trace
                // anywhere (not even a console entry), which is exactly what
                // made a real regression here look like expected empty state.
                // `Error` objects serialize to `{}` via structured/JSON clone
                // (message/stack aren't own-enumerable) — pull them out explicitly.
                const errInfo =
                    e instanceof Error ? { name: e.name, message: e.message, stack: e.stack } : { value: String(e) };
                Logger.error("agent", "MyAgentsList: ListRecentSessionsCommand failed", {
                    error: errInfo,
                    identityId: id,
                });
                if (myGeneration === fetchGeneration) setFetchError(true);
                return [];
            }
        }
    );

    // Mirror `rows()` into a keyed store so `<For>`/`<TransitionGroup>` can
    // tell "one row removed" from "list replaced" across a refetch.
    // `createResource`'s own signal hands back a brand-new array of
    // brand-new objects on every refetch, which `<For>`'s reference-based
    // reconciliation can't diff against the previous array at all — in
    // practice every row would unmount/remount on every "agents:changed"
    // refetch, not just the deleted one, which also defeats any per-row
    // exit animation. `reconcile(..., { key: "definition_id" })` is the
    // same idiom `drone-model.ts`'s `setDraft` already uses for its store.
    // See docs/specs/SPEC_AGENT_ROW_DELETE_ANIMATION_2026_09_16.md §4.1.
    const [rowsStore, setRowsStore] = createStore<{ list: RecentSessionRow[] }>({ list: [] });
    createEffect(() => {
        setRowsStore("list", reconcile(rows() ?? [], { key: "definition_id" }));
    });

    // definition_ids genuinely being deleted right now (added in
    // confirmDelete, before the row leaves rowsStore.list) — NOT every row
    // that happens to drop out of `sortedRows()`. A name-filter narrowing
    // the visible set, or a background refetch reconciling away a row for
    // some other reason, also removes it from the array `<TransitionGroup>`
    // watches; without this distinction the poof animation would fire for
    // ordinary filtering too, misrepresenting it as a delete (codex P2 on
    // PR #3280). Read by the TransitionGroup's onExit handler in my-agents-list-exit.ts (§4.2/
    // §4.3 of SPEC_AGENT_ROW_DELETE_ANIMATION_2026_09_16.md) to decide
    // whether an exiting row gets the real animation or an instant removal,
    // and by `hasPendingDeleteExit` to keep the list mounted (reagent P1 +
    // codex P2 on the same PR: the empty/no-match `<Show>` gate used to
    // unmount the whole `<ul>`/`TransitionGroup` — killing the animation
    // outright — the instant the last visible row's optimistic removal
    // made the list look empty).
    const [deletingIds, setDeletingIds] = createSignal<Set<string>>(new Set());
    const hasPendingDeleteExit = () => deletingIds().size > 0;

    // Re-poll on visibility regain so a session that just ended in
    // another pane shows up at the top without the user having to
    // re-open the picker. createEffect runs after first render too,
    // so the initial subscribe doesn't double-fetch.
    const onVisible = () => {
        if (document.visibilityState === "visible") void refetch();
    };
    document.addEventListener("visibilitychange", onVisible);
    onCleanup(() => document.removeEventListener("visibilitychange", onVisible));

    // Refetch when a new agent definition is created (e.g. via agent.define)
    // so the stub instance appears immediately without needing a restart.
    const unsubAgents = onMuxEvent(WpsEvent.AgentsChanged, () => void refetch());
    onCleanup(unsubAgents);

    // Surfacing rules:
    // - rows.loading              → still loading (skeleton hint) — uses the
    //                                RESOURCE's own loading flag, not
    //                                `rows() === undefined`: Solid's
    //                                createResource keeps the previous
    //                                value visible while a refetch is in
    //                                flight (stale-while-revalidate), so
    //                                after Retry on a failed fetch, `rows()`
    //                                is still `[]` from the failed attempt
    //                                even though a fresh request is
    //                                pending. Checking `rows() === undefined`
    //                                only catches the very first load — a
    //                                retry would fall straight through to
    //                                the empty branch below for its entire
    //                                in-flight duration, flashing "No agents
    //                                yet" and recreating the exact
    //                                error/empty ambiguity this fix exists
    //                                to remove (codex P2 on PR #2327's
    //                                post-merge re-review).
    // - fetchError()              → error state (retry affordance), never
    //                                confused with a genuinely empty list
    // - rows [] (fetch succeeded) → empty state (filter-aware copy)
    // - rows non-empty            → list
    //
    // `rowsStore.list`, not `rows()`, for the length checks below — reagent
    // P1 on PR #3280: confirmDelete's optimistic removal (§4.1 of
    // SPEC_AGENT_ROW_DELETE_ANIMATION_2026_09_16.md) empties
    // `rowsStore.list` immediately, but `rows()` itself stays stale until
    // the background "agents:changed" refetch resolves. Reading `rows()`
    // here mismatched isEmpty/isNoMatch against what filteredRows/
    // sortedRows (already on rowsStore.list) actually render — deleting
    // the last agent showed "No agents match" instead of the real empty
    // state for that window.
    const isLoading = () => rows.loading;
    const isEmpty = () => !isLoading() && !fetchError() && rowsStore.list.length === 0;

    // Report the first resolution up to the parent (see onFirstLoad's own
    // doc comment on MyAgentsListProps). `once` guards against a stale
    // closure re-firing `props.onFirstLoad` if it ever changed identity —
    // not expected from AgentPicker's call site today, but cheap insurance
    // since this must only ever fire once regardless.
    let firstLoadReported = false;
    createEffect(() => {
        if (rows() !== undefined && !firstLoadReported) {
            firstLoadReported = true;
            props.onFirstLoad?.();
        }
    });

    // Client-side name filter over the already-fetched page (bumped to
    // SEARCH_LIMIT while searching — see resourceKey/fetcher above).
    // Matches instance_name first, falling back to definition_name for
    // rows without a custom instance name (e.g. freshly-created agents).
    // SPEC_AGENT_PICKER_FILTER_SEARCH_2026_08_17.md. Literal first: names that
    // contain what was typed are listed, and only when none does does the
    // shared fuzzy search (SPEC_SETTINGS_PANE_SEARCH_2026_09_21.md §5) step in
    // for a typo. Fuzzy alone listed "agenty" for "agentx", which is wrong when
    // the user types a name they know. No `keywords` concept here (agent names
    // are free text, not a fixed vocabulary), so no synonym matching.
    const filteredRows = createMemo(() => {
        const q = nameQuery();
        // `rowsStore.list`, not `rows()` — see the reconcile effect above;
        // this is what gives `<For>`/`<TransitionGroup>` stable per-row
        // identity across a refetch.
        const all = rowsStore.list;
        if (!q) return all;
        const nameOf = (r: RecentSessionRow) => r.instance_name || r.definition_name;
        return literalFirstSearch(all, q, nameOf, { keys: [{ name: "searchName", getFn: nameOf }] });
    });
    // Distinct from `isEmpty()`: the fetch found real rows, but the
    // filter narrowed them all away — not "you have no agents."
    // `rowsStore.list`, not `rows()` — same reagent P1 fix as isEmpty above:
    // this must agree with what filteredRows() itself is derived from, or
    // an optimistic delete leaves this reading the stale pre-delete count.
    const isNoMatch = () => !isLoading() && !fetchError() && rowsStore.list.length > 0 && filteredRows().length === 0;

    // Sort applied AFTER filtering, over whatever's already in memory — no
    // backend RPC change (§3 of the audit report above). `.slice()` before
    // `.sort()` since Array.prototype.sort mutates in place and
    // `filteredRows()` may be the SAME array reference `rows()` returned
    // when there's no active name filter (see filteredRows's own `return
    // all` fast path) — sorting that in place would mutate the resource's
    // cached value out from under Solid's stale-while-revalidate.
    const sortedRows = createMemo(() => {
        const sort = props.sortBy?.() ?? "recent";
        return filteredRows()
            .slice()
            .sort((a, b) => compareRows(sort, a, b));
    });

    return {
        filterId,
        rawQuery,
        refetch,
        fetchError,
        setRowsStore,
        deletingIds,
        setDeletingIds,
        hasPendingDeleteExit,
        isEmpty,
        isNoMatch,
        filteredRows,
        sortedRows,
    };
}

export type MyAgentsData = ReturnType<typeof useMyAgentsData>;

// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * MyAgentsList — the top section of the two-tier AgentPicker
 * (SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md, Phase 1). Lists the user's
 * own agents (Maks, AgentY, etc.) — formerly known as "Recent sessions"
 * before the rename. Templates (Claude Code, Codex CLI, …) live in the
 * sibling Templates section below this one and are not surfaced here.
 *
 * Why "Recent sessions" became "My Agents" — under Option E session
 * zones are anchored to the agent definition, so a user agent has
 * exactly one "current" session by construction. The list is really
 * "your agents, with their current state", not "sessions across all
 * agents". Naming follows the new model.
 *
 * Renamed from RecentSessionsList (the cascade follow-up file
 * shipped in PR #977 + #1008). The reattach mechanism, data source, and
 * row UI are unchanged from that file — only the labels move. The data
 * source stays `ListRecentSessionsCommand` because the row UI uses
 * per-instance preview + node_count + last_active_at, which are only on
 * `RecentSessionRow` (NamedAgentRow doesn't carry them). Switching the
 * RPC would require backend changes and is out of Phase 1 scope.
 *
 * Reattach mechanism (unchanged): each entry triggers a normal
 * definition launch through `AgentViewModel.launchAgentDefinition` with
 * `continueOfInstanceId` + `workDirOverride` set from the row. The new
 * pane spawns the CLI in the prior working directory, so Claude's
 * `--continue` (and equivalents) resumes the session and the new pane's
 * `output.state.json` snapshot path picks up the conversation history
 * on render.
 *
 * Fork prompt (feat/agent-session-fork): when a row's definition is
 * already open in another pane, clicking it shows an inline prompt
 * instead of immediately reattaching. The user can either fork into a
 * new named session or switch focus to the existing pane.
 *
 * An agent that runs with no pane (always-available, parked) gets the same kind of
 * choice: "Open it" attaches it to this pane, "Fork" makes a named copy. Its row has a
 * dashed border and a "no pane" tag, so it doesn't look like any idle card.
 */
import { createMemo, onCleanup, For, Show, type Accessor, type JSX } from "solid-js";
import { TransitionGroup } from "solid-transition-group";
import { Button } from "@/app/element/ui";
import { shouldReduceMotion } from "@/app/motion/motion";
import { useTick } from "@/app/hook/useTick";
import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import type { RecentSessionRow } from "@/types/rpc/RecentSessionRow";
import type { OpenAgentLocation } from "../open-agent-panes";
import type { AgentSortOption } from "./AgentPickerFilterBar";
import { useMyAgentsData } from "./my-agents-list-data";
import { rowExitHandlers } from "./my-agents-list-exit";
import { createRowPanels } from "./my-agents-list-panels";
import { EMPTY_FILTERED, EMPTY_GLOBAL, FETCH_ERROR, noMatchText } from "./my-agents-list-text";
import { MyAgentsRow } from "./MyAgentsRow";
import { MyAgentsRowPanel } from "./MyAgentsRowPanel";

export { EMPTY_FILTERED, EMPTY_GLOBAL, FETCH_ERROR, noMatchText };

export interface MyAgentsListProps {
    /** Optional reactive accessor for the identity filter. `null` /
     *  undefined / empty string = no filter (show every identity). */
    identityId?: Accessor<string | null | undefined>;
    /** Visible cap. Backend caps at 100; default is 20. */
    limit?: number;
    /** Optional reactive accessor for a name filter (matched against
     *  `instance_name || definition_name`, case-insensitive substring).
     *  `null` / undefined / empty string = no filter (show every row).
     *  When non-empty, the fetch limit is bumped to `SEARCH_LIMIT` so the
     *  filter searches the full backend-capped set, not just the default
     *  page — see SPEC_AGENT_PICKER_FILTER_SEARCH_2026_08_17.md. */
    nameFilter?: Accessor<string | null | undefined>;
    /** Optional reactive accessor for the sort order — client-side, over
     *  the already-fetched/filtered row set (no backend RPC change).
     *  Defaults to `"recent"` (today's implicit backend ordering) when
     *  omitted. See docs/reports/REPORT_AGENT_PICKER_FIELD_ORDER_SORT_AND_DATA_GAPS_AUDIT_2026_08_24.md §3. */
    sortBy?: Accessor<AgentSortOption>;
    /** Called when the user clicks an entry that is NOT currently open
     *  in another pane. The parent (AgentPicker) hands this to
     *  `AgentViewModel.launchAgentDefinition` with the continuation
     *  overrides — see "Reattach mechanism" above. */
    onReattach: (row: RecentSessionRow) => void;
    /**
     * Map of definition_id → blockId for definitions currently open in
     * another pane. When a row's definition_id is in this map, clicking
     * the row shows the fork prompt instead of calling onReattach.
     */
    openDefinitions?: Accessor<Map<string, string>>;
    /**
     * definition_id → where that open pane is (this window or another),
     * for the active badge's tooltip and the "already open" prompt.
     * Optional: without it the wording stays "in another pane".
     */
    openLocations?: Accessor<Map<string, OpenAgentLocation>>;
    /**
     * A fresh, instance-wide read of `openDefinitions`, awaited on row click
     * before deciding between the "already open" prompt and a reattach — the
     * rendered map may not have srv's answer yet. Optional: without it the
     * click reads `openDefinitions` as before.
     */
    resolveOpenDefinitions?: () => Promise<Map<string, string>>;
    /**
     * Called after the user confirms a fork: fork has been created and
     * the new definition should be launched. Receives the new AgentDefinition.
     */
    onFork?: (row: RecentSessionRow, branchLabel: string) => Promise<void>;
    /**
     * Called when the user clicks "Switch to existing" in the fork
     * prompt. Receives the blockId of the already-open pane.
     */
    onSwitchToExisting?: (blockId: string) => void;
    /**
     * Called exactly once, the first time this component's own
     * ListRecentSessionsCommand resource resolves (success OR failure —
     * `rows()` transitions away from `undefined` either way). Lets a parent
     * that also has its own async gate (AgentPicker's `agents()` list) hold
     * a single combined loading overlay until BOTH are ready, instead of
     * this component's own empty-`<ul>`-while-loading state flashing
     * separately underneath. Never re-fires on background refetches
     * (visibility regain, agents:changed) — those correctly keep showing
     * the already-loaded list via Solid's stale-while-revalidate and have
     * nothing to do with "first load."
     */
    onFirstLoad?: () => void;
    /**
     * Row-menu "View History" action — opens a read-only history tab for
     * the row's agent. Delegates up to the parent (AgentPicker) because
     * `openOrFocusHistoryTab` needs the PICKER's OWN blockId (the tab this
     * list is rendered inside) to push the history tab onto, which this
     * component has no reason to hold itself.
     * docs/specs/SPEC_AGENT_DELETE_2026_09_16.md §4.3.
     */
    onViewHistory?: (row: RecentSessionRow) => void;
    /** The agent's runtime state (store/agent-runtime.ts), for the chip on its row. */
    runtimeOf?: (definitionId: string) => AgentRuntime | undefined;
    /**
     * Called when the user clicks an agent that runs with no pane shown anywhere: it attaches to the
     * process that is running, where `onReattach` would start another one.
     */
    onAttach?: (row: RecentSessionRow) => void;
}

export const MyAgentsList = (props: MyAgentsListProps): JSX.Element => {
    // The data (srv's rows, the filter, the sort, the states), the panel under a tile (its state and acts), and
    // the minute tick the times are read against.
    const data = useMyAgentsData(props);
    const panels = createRowPanels(props, data);
    const { filterId, rawQuery, refetch, fetchError, deletingIds, setDeletingIds, hasPendingDeleteExit, isEmpty, isNoMatch, filteredRows, sortedRows } = data;
    const exit = rowExitHandlers(deletingIds, setDeletingIds);
    const minuteTick = useTick(60_000);
    const now = createMemo(() => (minuteTick(), Date.now()));

    return (
        <div class="agent-recent-sessions" data-testid="agent-my-agents-list" ref={panels.setRootRef}>
            <div class="agent-recent-sessions-header">
                <span class="agent-recent-sessions-title">My Agents</span>
                {/* No `!isLoading()` guard here (reagent P2 on PR #2328):
                    that check was already redundant even before isLoading
                    switched to rows.loading — if rows() were undefined,
                    `.length > 0` is already false — but now that isLoading
                    also covers BACKGROUND refetches (visibility regain,
                    agents:changed events), keeping the guard would hide
                    the count on every one of those instead of just the
                    very first load, which is a real, visible flicker
                    regression this component never had before. Solid's
                    stale-while-revalidate means `rows()` keeps showing the
                    last real count during a background refetch anyway —
                    exactly what should render. */}
                <Show when={filteredRows().length > 0}>
                    <span class="agent-recent-sessions-count" data-testid="agent-my-agents-count">
                        {filteredRows().length}
                    </span>
                </Show>
            </div>
            <Show
                // `|| hasPendingDeleteExit()`: don't unmount the list (and
                // with it, the <TransitionGroup> mid-animation) just
                // because deleting the last visible row's optimistic
                // removal made it LOOK empty/no-match before the poof has
                // actually finished playing — reagent P1 + codex P2 on PR
                // #3280.
                when={(!isEmpty() && !fetchError() && !isNoMatch()) || hasPendingDeleteExit()}
                fallback={
                    <Show
                        when={fetchError()}
                        fallback={
                            <Show
                                when={isNoMatch()}
                                fallback={
                                    <div class="agent-recent-sessions-empty" data-testid="agent-my-agents-empty">
                                        {filterId() ? EMPTY_FILTERED : EMPTY_GLOBAL}
                                    </div>
                                }
                            >
                                <div class="agent-recent-sessions-empty" data-testid="agent-my-agents-no-match">
                                    {noMatchText(rawQuery())}
                                </div>
                            </Show>
                        }
                    >
                        <div class="agent-recent-sessions-error" data-testid="agent-my-agents-error">
                            <span class="agent-recent-sessions-error-msg">{FETCH_ERROR}</span>
                            <Button
                                tone="danger"
                                class="agent-recent-sessions-retry"
                                onClick={() => void refetch()}
                                data-testid="agent-my-agents-retry"
                            >
                                Retry
                            </Button>
                        </div>
                    </Show>
                }
            >
                <ul class="agent-recent-sessions-list" ref={panels.measureList}>
                    {/* Exit ("poof") + move (rubbery grid reflow) animation
                        on row delete. FLIP-based (measures rects, inverts,
                        plays) — see
                        docs/specs/SPEC_AGENT_ROW_DELETE_ANIMATION_2026_09_16.md
                        §4.2/§4.3. No enter animation (non-goal, §3 of that
                        spec) — enter* classes are explicitly empty.

                        Exit is handled IMPERATIVELY (onBeforeExit/onExit),
                        not via the declarative exit*Class props (left
                        empty) — codex P2 on PR #3280: `<TransitionGroup>`
                        treats ANY row dropping out of `sortedRows()` as an
                        "exit," including one that just got filtered out by
                        a name-search query, not only a real delete. Only a
                        `definition_id` present in `deletingIds` (set by
                        confirmDelete, BEFORE the row leaves rowsStore.list)
                        gets the real poof; everything else calls `done()`
                        immediately for an instant removal — the same
                        behavior this list always had for filtering, before
                        this animation existed. A single class
                        (`agent-row-exit-active`) is enough since the poof
                        is one self-contained `@keyframes` `animation`, not
                        a Vue-style enter/active/to transition dance.

                        `onBeforeExit` pins the exiting row's on-screen
                        width/height as inline styles before that class
                        (which switches it to `position: absolute`, pulling
                        it out of the CSS grid's flow so surviving rows can
                        immediately reflow) applies — without this the row
                        would collapse to its content's intrinsic width
                        instead of holding its original grid-cell size
                        while it animates. When the motion policy reduces
                        `transition` (dormant today), both handlers bail to
                        the instant-removal path; the stylesheet's reduced
                        treatment is a backstop. */}
                    <TransitionGroup
                        enterClass=""
                        enterActiveClass=""
                        enterToClass=""
                        moveClass={shouldReduceMotion("transition") ? "" : "agent-row-move"}
                        onBeforeExit={exit.onBeforeExit}
                        onExit={exit.onExit}
                    >
                        <For each={sortedRows()}>
                            {(row) => {
                                onCleanup(() => panels.tileEls.delete(row.definition_id));
                                return (
                                    <>
                                        <MyAgentsRow
                                            row={row}
                                            list={props}
                                            now={now}
                                            isExpanded={() => panels.isRowExpanded(row.definition_id)}
                                            onToggle={() => panels.toggleRowPanel(row)}
                                            tileRef={(el) => panels.tileEls.set(row.definition_id, el)}
                                        />
                                        <Show when={panels.panelAfter(row.definition_id)} keyed>
                                            {(open) => <MyAgentsRowPanel row={open} panels={panels} openLocations={props.openLocations} />}
                                        </Show>
                                    </>
                                );
                            }}
                        </For>
                    </TransitionGroup>
                </ul>
            </Show>
        </div>
    );
};

MyAgentsList.displayName = "MyAgentsList";

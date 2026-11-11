// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentComposerStrip — status row that sits directly above the textarea in
 * the agent pane composer region. As of Rev 7 (2026-08-26): a stats zone
 * (center, always true-centered) plus a list of ROWS, each with its own
 * left-anchored and right-anchored half — see `computeComposerRows` and
 * docs/specs/SPEC_COMPOSER_STRIP_ROW_BASED_LAYOUT_2026_08_26.md. The strip
 * grows from 1 row up to however many it actually needs as the pane
 * narrows, based on real rendered slot widths, not a guessed magic number
 * — but (unlike Rev 1-6) deciding HOW MANY rows are needed now also needs
 * the strip's own real available width (a `ResizeObserver`, see `stripWidth`
 * below), not purely CSS `@container` queries — see Rev 7's own paragraph
 * further down for why.
 *
 * Misc elements (everything except the centered stats zone) are pooled
 * into an ordered list of "slots" (see `slots` below), each carrying a
 * fixed semantic `side` — left is "what agent, what mode, how much context
 * is left" (the runtime trigger + the context group); right is "status
 * indicators + the one real action" (process badge, auth tag, then
 * NATIVE/CONTAINER+Shell, Shell always outermost) — used ONLY as the fallback
 * ordering when real measurement isn't available yet (see Rev 6/7's own
 * paragraphs). Once real widths ARE available, `rows()` computes the
 * actual left/right split and row grouping fresh, which can and does
 * override this fixed `side` pairing when real widths call for it.
 *
 * The row-layout logic and its history (Revs 4 to 9) are in composer-row-layout.ts.
 *
 * Stats zone (center, unaffected by the above): tokens (↑in ↓out) ·
 * elapsed.
 *
 * The strip bar itself is not clickable. "Shell" is the sole toggle for
 * the details drawer (the AgentShellSubblock terminal — "system"-tagged
 * activity-log lines write directly into it rather than a separate panel,
 * see useShellLogBridge.ts's `log`/`handleShellTermReady`).
 * Mode/Model/Effort used to be
 * three separate FlyoutMenu drop-up pills here (SPEC_COMPOSER_STRIP_MODE_TOPLEVEL_2026_07_02
 * Fix 7); they're now consolidated into one AgentRuntimeDropup trigger + panel
 * — see docs/specs/SPEC_AGENT_RUNTIME_DROPUP_2026_07_09.md.
 */

import type { AutoCompactPoint } from "@/app/store/agent-pane-state/auto-compact";
import type { CompactionState } from "@/app/store/agent-pane-state/types";
import { formatCompactNumber } from "@/util/format-count";
import { For, Show, createEffect, createMemo, createSignal, onCleanup, onMount, untrack, type Accessor, type JSX } from "solid-js";
import { Button } from "@/app/element/ui/Button";
import { AgentSessionStats } from "./AgentSessionStats";
import type { SessionStats } from "../../../agent-model/types";
import { AgentRuntimeDropup } from "./AgentRuntimeDropup";
import { RuntimeBadge } from "./RuntimeBadge";
import { shortenEmail } from "@/app/view/identity/identity-model";
import { compactionCountdownText, contextTitle, ctxBand } from "./context-bandwidth";
import { computeComposerRows, computeStatsInline, orderKeysForEdgePriority, rowsCost, type ComposerRow } from "./composer-row-layout";
import { useBlockStats } from "@/app/hook/useBlockStats";
import { BlockStatsBadge } from "@/element/blockstats";
import { ACCOUNT_BUDGETS, accountInitial, middleTruncate, shortContextText } from "./composer-forms";
import { useSlotForms } from "./composer-forms-probes";
import { useStatsZone } from "./composer-stats-zone";
import { isContainerAgentMode, parseRuntimeKind } from "@/util/runtime-kind";
import { tip } from "@/app/element/tip";

// ── Props ──────────────────────────────────────────────────────────────────

interface AgentComposerStripProps {
    /** True while a turn is in flight. */
    loading?: boolean;
    /** Reducer-projected: activity log panel open/closed. */
    logOpen: boolean;
    /** Dispatches `DetailsToggle` to the pane reducer. */
    onToggleLog: () => void;
    /** Tokens in context: the main agent's latest call's whole prompt — a
     *  reading that already passed `plausibleReading` (context-reading.ts). */
    contextTokens?: number | null;
    /** How many images the conversation holds, for the Runtime panel's model list. */
    conversationImages?: () => number;
    /** The model's context window. undefined = unknown (the strip then shows
     *  the token count alone, with no fill band). */
    contextWindow?: number;
    /** Where the two numbers come from (`contextReadingNote`), appended to
     *  the reading's tooltip and shown in its popover. */
    contextNote?: string;
    /** Where auto-compaction happens (auto-compact.ts): the CLI's own point,
     *  "off", or — when absent — the default window − 33K, assumed. */
    autoCompact?: AutoCompactPoint | null;
    /** Resolved model id on the last main-agent reply (the Runtime menu shows it). */
    lastReplyModel?: string | null;
    /** Durable logged-in/out state (useAgentControllerStatus's authStatus) —
     *  rendered as a small red/green tag right of the context text. Hidden
     *  entirely for "unknown" (before the first auth check resolves), so the
     *  strip doesn't flash a wrong color for an instant on every mount. */
    authStatus?: "authenticated" | "unauthenticated" | "unknown";
    /** Login email of the account this agent is bound to, when the provider
     *  recorded one (the account's `context.email`). Replaces "Logged in" with the
     *  address, shortened to 22 characters (`shortenEmail`); the tooltip keeps
     *  it whole. Ignored unless `authStatus` is "authenticated" (or see `signInCheckedLater`). */
    authEmail?: string;
    /** A container agent: its sign-in is checked when a turn starts, so `authStatus` stays "unknown"
     *  and the chip still shows the account (when known and switchable). */
    signInCheckedLater?: boolean;
    /** Whether this agent has another account to switch to (same provider,
     *  signed in — `computeAccountBindCandidates`). While signed in, idle, and
     *  true, the chip is a link that calls `onSwitchAccount`; otherwise it stays
     *  plain text. SPEC_COMPOSER_ACCOUNT_SWITCH_AND_JEKT_HEIGHT_CAP_2026_09_26.md. */
    canSwitchAccount?: boolean;
    /** Opens the account picker at the click. A switch restarts the agent, so
     *  the chip only offers it when no turn is running (`loading`) and no
     *  compaction is in progress (`compacting`). */
    onSwitchAccount?: (e: MouseEvent) => void;
    /** The account a queued switch moves to when the turn ends. */
    pendingAccountLabel?: string;

    /** Cumulative cost/tokens/duration for this pane, shown in the
     *  session-stats popover the context reading opens. */
    sessionTotals?: SessionStats | null;

    // ── Inline model / effort controls ────────────────────────────
    /** Block id — needed for applyRuntimeChange. */
    blockId?: string;
    /** Block atom — reads current model/effort from meta. */
    blockAtom?: () => Block | undefined;
    /** Provider id — needed for applyRuntimeChange. */
    providerId?: string;
    /** `block.meta["agentMode"]` — "host" or "container". Drives the compact
     *  NATIVE/CONTAINER tag next to the model selector (replaces the old,
     *  confirmed-inert "Host — full system access" / "Container — isolated
     *  Docker sandbox" pane row — see
     *  docs/specs/SPEC_AGENT_PANE_SCROLL_FOLLOW_AND_STATUS_OVERLAY_2026_07_24.md §3.2). */
    agentMode?: string;
    /**
     * Live "compaction in progress" state (reducer-owned, set by the
     * `PreCompact` hook's `compaction_started` signal). Gates the manual
     * Compact button (`canCompact()` below) and its tooltip. The
     * "Compacting…" + live elapsed readout this used to also drive here
     * moved to `AgentWorkingRow` (2026-08-27, Part 2 of
     * SPEC_REMOVE_AGENT_UNRESPONSIVE_DETECTION_2026_08_25.md) — see
     * `agent-view.tsx`'s `<AgentWorkingRow compacting=.../>` call site.
     */
    compacting?: CompactionState | null;
    /**
     * Manually trigger compaction now, instead of waiting for the CLI's
     * own auto-compact threshold. Claude and Codex (srv compacts a Codex
     * thread on "/compact", subprocess/codex_compact.rs). Sends the
     * literal text "/compact" through the normal send-message path
     * (agent-view.tsx's handleSendMessage), which the CLI's persistent
     * stream-json stdin protocol recognizes as its real /compact command
     * (verified empirically against a live `claude` process — see
     * docs/reports/REPORT_TOKEN_ACCOUNTING_AND_COMPACTION_CONTROL_2026_08_18.md
     * §6). Deliberately NOT routed through the SlashCommand registry
     * (commands/providers/claude.ts) — a REGISTERED command's handler
     * returning `{kind:"passthrough"}` is a documented no-op in
     * dispatch.ts's `formatResult` ("passthrough from a handler is a
     * noop here... ignored"), so registering `/compact` there would
     * actually swallow it instead of forwarding it. Leaving it
     * unregistered means dispatchSlashCommand's own top-level
     * unknown-command path returns passthrough BEFORE calling any
     * handler, which useAgentCommands.sendMessage correctly forwards
     * as a real turn — the same path a user manually typing "/compact"
     * already takes today.
     */
    onCompact?: () => void;
    /** Compacts the conversation, then moves the agent to the model on the route: the model panel's rows for a model too small for it. */
    onCompactThenSwitch?: (route: string, model: string) => Promise<void>;
    /** The View chip's text (the pane's view: "Standard", "Replies · this pane"); none while the pane shows everything. */
    viewChip?: string | null;
    /** The View popover is open, for the chip's expanded state. */
    viewOpen?: boolean;
    onOpenView?: () => void;
}

type Side = "left" | "right";
/** `compact`: the slot has a compact form, which `render` draws while its second argument says so (composer-forms.ts). */
type Slot = {
    key: string;
    side: Side;
    interactive: boolean;
    compact?: boolean;
    /** Character budgets between the full and compact forms, widest first (composer-forms.ts `ACCOUNT_BUDGETS`). */
    budgets?: readonly number[];
    render: (rowSide: Side, compact: Accessor<boolean>, budget?: Accessor<number | undefined>) => JSX.Element;
};

/** The pieces of the props the slots and the layout both read. */
interface StripDerived {
    showControls: () => boolean;
    ctxClass: () => string;
    hasSessionToManage: () => boolean;
    ctxText: () => string | null;
    ctxCountdownText: () => string | null;
    offersCompact: () => boolean;
    canCompact: () => boolean;
}

/** The elements the measurement reads; assigned during render (`let`s before the split). */
interface StripRefs {
    stripRef: HTMLDivElement | undefined;
    rowsRef: HTMLDivElement | undefined;
    // Rev 6 — real measured widths per slot, keyed by slot.key. Populated
    // by the measuring effect from each slot's `display: contents` ref
    // wrapper (see the render output); a plain mutable object, not a
    // signal, since it's only ever read inside that same effect right
    // after the DOM it describes has committed — no other reactive
    // consumer needs it directly.
    measureRefs: Record<string, HTMLElement | undefined>;
}

function createStripDerived(props: AgentComposerStripProps): StripDerived {
    // Mode · Model · Effort for Claude and Codex (Codex's own models, levels and sandbox
    // modes, runtime-capabilities.ts). Others have their own model lists — spec §1.3.
    const showControls = () => props.blockAtom != null && (props.providerId === "claude" || props.providerId === "codex");

    // Context text color based on proximity to compaction threshold.
    const ctxClass = (): string => {
        const t = props.contextTokens;
        const w = props.contextWindow;
        if (t == null || t <= 0 || w == null) return "";
        const b = ctxBand(t, w, props.autoCompact);
        return `agent-composer-strip-ctx--${b}`;
    };

    // Archive/Export live behind the ctx reading's popover, but the reading
    // itself only exists once a turn has reported context usage. A pane with
    // history and no reading yet — exactly the state the interrupted /
    // resume-failed banners describe — would otherwise have no way to reach
    // session management at all, which `AgentControlBar` always offered
    // (ReAgent P1 on #3436). In that case the slot still renders, labelled
    // "session" instead of a token count.
    const hasSessionToManage = (): boolean => {
        if (props.providerId !== "claude") return false;
        const meta = props.blockAtom?.()?.meta;
        if (!meta) return false;
        const lines = (meta["session:line_count"] as number | undefined) ?? 0;
        const archivedAt = (meta["session:archived_at"] as number | undefined) ?? 0;
        return lines > 0 || archivedAt > 0;
    };

    const ctxText = (): string | null => {
        const t = props.contextTokens;
        const w = props.contextWindow;
        if (t == null || t <= 0) return null;
        if (w == null) return `${formatCompactNumber(t)} ctx`;
        return `${formatCompactNumber(t)} / ${formatCompactNumber(w)}`;
    };

    // Tier 3 — explicit countdown, visible inline (not hover-gated) once
    // the fill level is worth calling out. null below the mid band, or
    // whenever the window itself is unknown (nothing to count down from).
    //
    // Gated to Claude only (Codex P2 on PR #2729): `compactionThreshold()`
    // hard-codes Claude Code's own ~33K auto-compact buffer
    // (context-window.ts's own doc comment). A non-Claude provider that
    // happens to report Claude-shaped `message_start` usage (e.g. a
    // `muxcode`-catalog entry) would otherwise get an invented countdown
    // against a threshold that has never been verified for it — the same
    // reason `canCompact()` below already restricts the manual-compact
    // button to Claude.
    const ctxCountdownText = (): string | null => {
        const t = props.contextTokens;
        const w = props.contextWindow;
        if (t == null || t <= 0 || w == null) return null;
        if (props.providerId !== "claude") return null;
        return compactionCountdownText(t, w, props.autoCompact);
    };

    // Manual compaction: Claude, and Codex on the host only (onCompact's doc), once there's
    // something to compact (Codex: a thread; it reports no context), never mid-turn or twice.
    const offersCompact = (): boolean =>
        props.onCompact != null && (props.providerId === "claude" || (props.providerId === "codex" && !isContainerAgentMode(props.agentMode)));
    const canCompact = (): boolean => {
        if (!offersCompact() || props.loading || props.compacting) return false;
        if (props.providerId === "codex") return !!props.blockAtom?.()?.meta?.["agent:sessionid"];
        return ctxText() != null;
    };
    return { showControls, ctxClass, hasSessionToManage, ctxText, ctxCountdownText, offersCompact, canCompact };
}

function runtimeSlot(props: AgentComposerStripProps): Slot {
    return {
        key: "runtime",
        side: "left",
        interactive: true,
        render: () => (
            <AgentRuntimeDropup
                blockId={props.blockId ?? ""}
                blockAtom={props.blockAtom ?? (() => undefined)}
                providerId={props.providerId ?? ""}
                lastReplyModel={props.lastReplyModel}
                contextTokens={props.contextTokens}
                conversationImages={props.conversationImages}
                compactThenSwitch={props.onCompactThenSwitch}
            />
        ),
    };
}

/** A container agent that is signed in elsewhere still shows its account, when it is known and switchable. */
const accountKnown = (props: AgentComposerStripProps): boolean =>
    props.authStatus === "unknown" && !!props.signInCheckedLater && !!props.canSwitchAccount && !!props.onSwitchAccount;

function authSlot(props: AgentComposerStripProps): Slot {
    return {
        key: "auth",
        // "right" until Rev 9 (2026-09-16); flipped to match the
        // pinned auth=left/ctx=right invariant `pinnedPairs`
        // establishes in the measured path — this `side` field is
        // the UNMEASURED fallback (first paint, JSDOM, or a shed
        // zero-width slot's fallback row), and leaving it at the
        // old value would have reintroduced exactly the swap this
        // revision fixes, just at the mount-time boundary instead
        // of the resize one (reagent P2, PR #3282). See
        // docs/specs/SPEC_COMPOSER_STRIP_AUTH_COMPACT_SIDE_STABILITY_2026_09_16.md.
        side: "left",
        // Interactive whenever the chip CAN be a "switch account"
        // link. Static, not tied to whether it is one right now (idle,
        // has candidates): the flag feeds edge-priority ordering, and
        // flipping it with every turn would reorder the strip.
        interactive: true,
        compact: true,
        budgets: ACCOUNT_BUDGETS,
        render: (_rowSide, compact, budget) => {
            const authed = () => props.authStatus === "authenticated";
            const hasOthers = () => !!props.canSwitchAccount && !!props.onSwitchAccount;
            // srv holds a switch asked for in a turn until it ends (agent.account.switch).
            const busy = () => !!props.loading || props.compacting != null;
            const canSwitch = () => hasOthers();
            // `cut` shortens an address: the full form cuts it to 22 characters, the budgeted form starts from the
            // whole address, so an address is cut once, not twice.
            const labelWith = (cut: (email: string) => string) => {
                const base = accountKnown(props) ? (props.authEmail ? cut(props.authEmail) : "Account") : !authed() ? "Not logged in" : props.authEmail ? cut(props.authEmail) : "Logged in";
                return props.pendingAccountLabel ? `${base} → ${cut(props.pendingAccountLabel)}` : base;
            };
            const label = () => labelWith(shortenEmail);
            // Tight: the dot and one letter, the whole label in the tooltip and the accessible name. Signed out
            // keeps a mark of its own, as "N" for "Not logged in" would read as a name.
            // With room to spare it shows the address cut in the middle instead ("taylo…@mail.dev"), by the budget
            // the strip gives it; a switch pending ("a@x → b@y") has no middle form and keeps the letter.
            const shown = () => {
                if (!compact()) return label();
                if (props.authStatus === "unauthenticated") return "!";
                const room = props.pendingAccountLabel ? undefined : budget?.();
                return room === undefined ? accountInitial(label()) : middleTruncate(labelWith((email) => email), room);
            };
            const title = () => {
                if (!authed() && !accountKnown(props)) return hasOthers() ? "Not signed in — click to log in or switch account" : "Not signed in — click Log in to continue";
                const base = accountKnown(props)
                    ? (props.authEmail ? `Account ${props.authEmail}` : "No account linked yet")
                    : props.authEmail
                    ? `Signed in as ${props.authEmail}`
                    : "Signed in to this agent's provider";
                if (props.pendingAccountLabel) return `${base}. Switching to ${props.pendingAccountLabel} when this turn ends.`;
                if (!hasOthers()) return base;
                return busy()
                    ? `${base}. Click to switch account; it takes effect when the current turn ends.`
                    : `${base}. Click to switch account.`;
            };
            return (
                <span
                    class="agent-composer-strip-auth"
                    classList={{
                        "agent-composer-strip-auth--ok": authed(),
                        "agent-composer-strip-auth--bad": props.authStatus === "unauthenticated",
                    }}
                    {...tip(title())}
                >
                    <span class="agent-composer-strip-auth-dot" aria-hidden="true" />
                    <Show when={canSwitch()} fallback={
                            <span role={compact() ? "img" : undefined} aria-label={compact() ? label() : undefined}>
                                {shown()}
                            </span>
                        }>
                        <Button
                            variant="link"
                            class="agent-composer-strip-auth-link"
                            aria-haspopup="menu"
                            aria-label={compact() ? label() : undefined}
                            onClick={(e) => props.onSwitchAccount?.(e)}
                        >
                            {shown()}
                        </Button>
                    </Show>
                </span>
            );
        },
    };
}

function ctxSlot(props: AgentComposerStripProps, d: StripDerived): Slot {
    return {
        key: "ctx",
        // ctx text + countdown (conditional) + Compact button
        // (conditional) render together as one unit — this slot
        // can't be split across zones, since Compact must stay
        // immediately adjacent to the context text (on the OUTER
        // side of it — see render(rowSide) below). Grouped with the
        // runtime trigger on the LEFT (2026-08-25, Rev 5) — this
        // slot alone can render 3 sub-elements, more than
        // everything else in the pool combined in the common
        // case, so pairing it with badge/auth/hostShell on the
        // right (as Rev 4 first shipped) put most of the strip's
        // visual weight on one side every time a Claude agent had
        // context tracking active. "Mode + context awareness"
        // (left) / "status indicators + the action button"
        // (right) is the resulting split — see
        // docs/specs/SPEC_COMPOSER_STRIP_DYNAMIC_BALANCE_2026_08_24.md
        // Rev 5. Flipped to "right" as of Rev 9 (2026-09-16) — see
        // the `auth` slot's own comment above; this is the
        // matching other half of the same fallback-side fix.
        side: "right",
        // Always interactive: the context reading itself is a button
        // that opens the session-stats popover (AgentSessionStats),
        // so this slot has something clickable even when the Compact
        // button doesn't render. It was gated on Compact back when
        // the reading was inert text.
        interactive: true,
        compact: true,
        // Compact sits on the OUTER end of whichever side this
        // slot renders on (edge priority): first on the left,
        // last on the right. The text + countdown stay adjacent
        // in reading order either way.
        render: (rowSide, compact) => {
            const compactBtn = () => (
                <Show when={d.offersCompact()}>
                    <Button
                        density="compact"
                        class="agent-composer-strip-compact-btn"
                        data-strip-button
                        icon={compact() ? "compress" : undefined}
                        aria-label={compact() ? "Compact" : undefined}
                        disabled={!d.canCompact()}
                        title={
                            d.canCompact()
                                ? "Summarize and trim this session's history now, instead of waiting for the CLI's own auto-compact threshold."
                                : props.compacting
                                    ? "Already compacting…"
                                    : "Wait for the current turn to finish before compacting."
                        }
                        onClick={() => props.onCompact?.()}
                    >
                        {compact() ? undefined : "Compact"}
                    </Button>
                </Show>
            );
            return (
                <>
                    {rowSide === "left" && compactBtn()}
                    <AgentSessionStats
                        blockId={props.blockId ?? ""}
                        blockAtom={props.blockAtom ?? (() => undefined)}
                        providerId={props.providerId ?? ""}
                        contextTokens={props.contextTokens}
                        contextWindow={props.contextWindow}
                        contextNote={props.contextNote}
                        sessionTotals={props.sessionTotals}
                        ctxClass={d.ctxClass()}
                        title={
                            props.contextTokens != null
                                ? contextTitle(props.contextTokens, props.contextWindow, props.contextNote, props.autoCompact) +
                                  // The countdown is not drawn when tight: the tooltip has it.
                                  (compact() && d.ctxCountdownText() ? ` ${d.ctxCountdownText()}` : "")
                                : undefined
                        }
                        label={compact() ? shortContextText(d.ctxText() ?? "session") : (d.ctxText() ?? "session")}
                    />
                    <Show when={!compact() && d.ctxCountdownText()}>
                        <span
                            class={`agent-composer-strip-ctx-countdown ${d.ctxClass()}`}
                            classList={{ "agent-composer-strip-ctx-countdown--critical": d.ctxClass().endsWith("critical") }}
                            {...tip("Applies to auto-compaction only — a manual /compact can happen at any fill level.")}
                        >
                            {d.ctxCountdownText()}
                        </span>
                    </Show>
                    {rowSide === "right" && compactBtn()}
                </>
            );
        },
    };
}

/**
 * The pane's live CPU and memory readout, next to the context reading. A status, not a control: the full
 * "0.0% 66M" while the row has room, a processor icon and the CPU figure when it is tight (the whole reading is
 * its tooltip, and the chip takes focus). The strip adds the slot once the first figures arrive.
 */
function statsSlot(props: AgentComposerStripProps): Slot {
    return {
        key: "stats",
        side: "right",
        interactive: false,
        compact: true,
        render: (_rowSide, compact) => (
            <BlockStatsBadge blockId={props.blockId ?? ""} placement="inline" form={compact() ? "compact" : "full"} />
        ),
    };
}

function viewSlot(props: AgentComposerStripProps): Slot {
    return {
        key: "view",
        side: "right",
        interactive: true,
        compact: true,
        // Tight: an eye, the chip's text as its tooltip and accessible name.
        render: (_rowSide, compact) => (
            <Button
                variant="link"
                class="agent-composer-strip-view"
                data-strip-button
                data-agent-view-chip
                icon={compact() ? "eye" : undefined}
                aria-label={compact() ? (props.viewChip ?? undefined) : undefined}
                aria-haspopup="dialog"
                aria-expanded={props.viewOpen ? "true" : "false"}
                title={compact() ? `${props.viewChip}. What this pane shows: click to change` : "What this pane shows: click to change"}
                onClick={() => props.onOpenView?.()}
            >
                {compact() ? undefined : props.viewChip}
            </Button>
        ),
    };
}

function hostShellSlot(props: AgentComposerStripProps): Slot {
    return {
        key: "hostShell",
        side: "right",
        interactive: true,
        compact: true,
        // Shell (the button) on the OUTER end, NATIVE badge (passive)
        // inward — mirrored by side the same way ctx's Compact is.
        // On the right this is the existing "Shell always outermost"
        // order, unchanged; the flip only shows in the degenerate
        // case where this slot is the line's sole (left) occupant.
        //
        // No wrapper span (element-level edge priority, 2026-08-26):
        // badge and button render as direct siblings in the slot's
        // measure wrapper — same shape as the ctx slot's elements —
        // so the SCSS edge-priority `order` rules can place the
        // passive badge inward past a NEIGHBORING slot's interactive
        // element (Compact), which a single wrapping flex item never
        // allowed. The user explicitly chose this over the older
        // "NATIVE just left of Shell" pairing when the two directives
        // collided (see _composer-strip.scss's removal note).
        render: (rowSide, compact) => {
            const hostBadge = () => (
                <Show when={parseRuntimeKind(props.agentMode) !== undefined}>
                    <RuntimeBadge runtime={props.agentMode!} size="tag" />
                </Show>
            );
            const shellBtn = () => (
                <Button
                    tone="accent"
                    density="compact"
                    class="agent-composer-strip-log-btn"
                    pressed={props.logOpen}
                    icon={compact() ? "terminal" : undefined}
                    aria-label={compact() ? "Shell" : undefined}
                    title={props.logOpen ? "Hide the shell" : "Show the shell"}
                    onClick={() => props.onToggleLog()}
                >
                    {compact() ? undefined : "Shell"}
                </Button>
            );
            return (
                <>
                    {rowSide === "left" && shellBtn()}
                    {hostBadge()}
                    {rowSide === "right" && shellBtn()}
                </>
            );
        },
    };
}

// Rev 7 — the strip's own real available width, needed to decide
// single-row vs. multi-row (see `createLayout` below and
// docs/specs/SPEC_COMPOSER_STRIP_ROW_BASED_LAYOUT_2026_08_26.md §3.3).
// A genuinely new capability this file didn't need before Rev 7 —
// every prior revision relied purely on CSS `@container` queries.
// Guarded (`typeof ResizeObserver === "undefined"`) the same way
// every other ResizeObserver use in this codebase is (JSDOM, this
// file's own unit-test environment, has no ResizeObserver at all) —
// `rows()` below already falls back to a single fixed-side row
// whenever `stripWidth()` is still 0, so skipping the observer
// entirely in that environment is correct, not just tolerated.
//
// Observes `.agent-composer-strip-rows` — NOT the outer
// `.agent-composer-strip` (see `refs.stripRef` below) — for two reasons,
// both found via real dev-build screenshot verification at the narrow
// tier (task #48), not by inspection alone:
//
//   - `entry.contentRect` is reported in the element's own LOCAL CSS
//     pixels (unaffected by any CSS `zoom` on an ancestor — e.g. the
//     agent view's own `zoom: 0.8`), while the per-slot widths
//     measured below use `getBoundingClientRect()`, which IS
//     zoom-scaled (viewport-relative). Comparing those two units
//     directly made the "does everything fit on one line" check
//     systematically over-generous under any non-1 ancestor zoom.
//     Reading `getBoundingClientRect()` off the observed element
//     itself instead keeps both sides of that comparison in the same
//     coordinate space regardless of ancestor zoom.
//   - `.agent-composer-strip`'s own `getBoundingClientRect().width` is
//     its BORDER-BOX width, which includes its own horizontal padding
//     (`padding: var(--space-1) var(--space-2)`, 16px total) — slot
//     widths have no padding component, so comparing them against
//     that inflated width overstated how much fits on one line the
//     same way the zoom mismatch above did (reagent P1, PR #2812).
//     `.agent-composer-strip-rows` has no padding/border of its own,
//     so its border-box and content-box widths are identical — its
//     rect gives the real available content width directly, still
//     via `getBoundingClientRect()` for the zoom-consistency reason
//     above.
function useStripWidth(refs: StripRefs): Accessor<number> {
    const [stripWidth, setStripWidth] = createSignal(0);
    onMount(() => {
        if (!refs.rowsRef || typeof ResizeObserver === "undefined") return;
        const ro = new ResizeObserver((entries) => {
            for (const entry of entries) {
                setStripWidth(entry.target.getBoundingClientRect().width);
            }
        });
        ro.observe(refs.rowsRef);
        onCleanup(() => ro.disconnect());
    });
    return stripWidth;
}

// A `getComputedStyle(...)`-read length (e.g. a `column-gap` or CSS
// custom property) is reported in the element's own LOCAL CSS pixels,
// unaffected by an ancestor's CSS `zoom` — the same property that made
// `entry.contentRect` wrong above, for the same reason. Every width in
// this file that feeds a fit/pairing decision is measured via
// `getBoundingClientRect()` instead, which IS zoom-scaled. A raw
// `getComputedStyle` gap value mixed directly into one of those totals
// re-introduces the exact zoom-unit mismatch this file's own Rev 7
// fix eliminated for width — just via the gap term instead (found via
// a real regression report after PR #2812 shipped: the widest tier
// started splitting into 2 lines, and the narrow tier grew MORE
// one-sided rows than necessary, both explained by systematically
// overestimating how much space gaps need). `zoom` applies uniformly
// to a whole subtree, so the ratio measured off ANY element inside it
// (here, the always-mounted `stripRef`) is the correct multiplier for
// a `getComputedStyle` length read anywhere else in that same
// subtree — multiply by this wherever the two measurement styles mix.
const zoomRatio = (refs: StripRefs): number => {
    if (!refs.stripRef || !refs.stripRef.clientWidth) return 1;
    return refs.stripRef.getBoundingClientRect().width / refs.stripRef.clientWidth;
};

/** The column-gap between a row's slots, in the same zoomed pixels as the measured widths. */
const stripGap = (refs: StripRefs): number =>
    refs.rowsRef ? (parseFloat(getComputedStyle(refs.rowsRef).getPropertyValue("--space-2")) || 8) * zoomRatio(refs) : 8;

// Re-measures every current slot whenever the pool changes shape OR
// any slot's own content changes width (both flow through `slots()`
// recomputing — e.g. ctx text ticking with token counts). Each
// child's OWN width is zone-independent (neither -controls nor
// -right applies any width-affecting rule to children beyond
// flex-wrap), but the GAP BETWEEN a multi-child slot's own children
// (e.g. ctx's 3 sub-elements, hostShell's 2) is not — reagent P2 on
// PR #2808: summing only `getBoundingClientRect().width` ignored the
// real `gap` the current zone applies between them, systematically
// under-measuring multi-child slots by ~1-2 gaps' worth of pixels,
// comparable in size to the 8px rounding bucket below. Reading the
// wrapper's own PARENT's computed `column-gap` (not a hardcoded
// `--space-1`/`--space-1-5` constant, which would silently drift if
// the SCSS values ever change) gets the real value for whichever
// zone this slot currently sits in. This converges to a stable split
// after one settle pass instead of oscillating between two different
// "correct" widths.
//
// Also depends on `stripWidth()` — regression found post-merge: a
// slot's OWN rendered width isn't purely a function of its content;
// this file's own SCSS shed-content query (`.agent-composer-strip-
// auth` collapsing to
// `display:none` below a container-width threshold) makes it a
// function of the CONTAINER width too. Without this dependency, a
// pure resize crossing a shed threshold never re-ran this effect —
// `slots()` (prop-driven) hadn't changed — so `slotWidths` kept
// whatever a slot's width was BEFORE it got shed, indefinitely, until
// some unrelated prop happened to retrigger a remeasurement. The
// Codex P1 shed-slot fix (which excludes zero-width slots from
// pairing) never even ran for a slot that measurement never told it
// was zero — the exact same "stale measurement crosses a structural
// transition" failure mode already fixed for `statsWidth` above, one
// more measurement path where it had gone unnoticed.
function useSlotWidths(slots: Accessor<Slot[]>, stripWidth: Accessor<number>, refs: StripRefs): Accessor<Record<string, number>> {
    const [slotWidths, setSlotWidths] = createSignal<Record<string, number>>({});
    createEffect(() => {
        stripWidth();
        const keys = slots().map((s) => s.key);
        const widths: Record<string, number> = {};
        for (const key of keys) {
            const el = refs.measureRefs[key];
            if (!el) continue;
            const children = Array.from(el.children);
            let total = 0;
            for (const child of children) {
                total += child.getBoundingClientRect().width;
            }
            if (children.length > 1 && el.parentElement) {
                // `* zoomRatio()` — see that function's own doc comment;
                // `columnGap` is a `getComputedStyle` (local/unzoomed)
                // read, but `total` above is built from
                // `getBoundingClientRect()` (zoomed) sums.
                const gapPx = (parseFloat(getComputedStyle(el.parentElement).columnGap) || 0) * zoomRatio(refs);
                total += gapPx * (children.length - 1);
            }
            // Rounded UP to an 8px bucket — the live turn ticker (elapsed
            // time, token counts) can shift a slot's text width by a
            // pixel or two every second; without bucketing, the balance
            // below could flip-flop every tick even though nothing
            // meaningful about the content actually changed. `ceil`, not
            // `round` (user-reported regression, 2026-08-26): nearest-8
            // can UNDER-report a slot by up to 4px, making the fit check
            // in `computeComposerRows` optimistic — the row's own CSS
            // flex-wrap safety net then overflows a couple of pixels
            // BEFORE the JS split threshold, so a resize shows two
            // distinct layout breaks within a few px of each other.
            // Rounding up keeps the anti-jitter bucketing while
            // guaranteeing the JS decision always fires at-or-before the
            // point real layout would overflow.
            widths[key] = Math.ceil(total / 8) * 8;
        }
        setSlotWidths(widths);
    });
    return slotWidths;
}

// reagent P1 on PR #2808: `<For>` (below) reconciles by referential
// identity of each item in its `each` array — passing `slots()`
// entries directly gives every one a brand-new `{key, side, render}`
// object on EVERY recompute (any authStatus/ctxText
// change, which ticks every second during an active turn). With no
// stable identity to compare against, `<For>` would treat that as
// "every slot removed and re-added," remounting `AgentRuntimeDropup`
// — which owns its own `open`/`selectedOptIndex` signals — and
// silently collapsing an open Mode/Model/Effort dropdown on a
// completely unrelated slot's change. Plain string keys compare by
// VALUE, not reference, so `<For>` over row-derived key arrays (see
// `rows` below) correctly preserves DOM/component identity for any
// slot whose row/side didn't change. `slotByKey` is read once inside
// each `<For>` item's own template callback via `untrack` — Solid
// only re-invokes that callback when the KEY ITSELF changes, not on
// every parent recompute, and `untrack` stops the LOOKUP itself from
// being a tracked dependency of that position (a bare
// `{slotByKey().get(key)?.render()}` would still re-invoke `render()`
// on every `slots()` change even with a stable key, defeating the
// whole point) — so the returned JSX stays reactive to that slot's
// OWN signal reads regardless (the standard `<For>` idiom). A slot
// that genuinely changes row or side (a real rebalance decision, not
// an unrelated prop change) still gets destroyed in one `<For>` and
// recreated in the other — an inherent limit of separate `<For>`
// blocks, not something a key can paper over, but a real row/side
// change is a far rarer event than "any slot's content changed."
function createSlotMap(slots: Accessor<Slot[]>): Accessor<Map<string, Slot>> {
    return createMemo(() => {
        const map = new Map<string, Slot>();
        for (const s of slots()) map.set(s.key, s);
        return map;
    });
}

/** Edge priority: reorders a row's keys so a slot you click sits on the outer end (see `orderKeysForEdgePriority`). */
function edgeOrderer(list: Slot[]): (r: ComposerRow) => ComposerRow {
    const interactiveByKey = new Map(list.map((s) => [s.key, s.interactive]));
    const edgeOrdered = (r: ComposerRow): ComposerRow => ({
        left: orderKeysForEdgePriority(r.left, "left", (k) => interactiveByKey.get(k) ?? false),
        right: orderKeysForEdgePriority(r.right, "right", (k) => interactiveByKey.get(k) ?? false),
    });
    return edgeOrdered;
}

/**
 * The single fixed-side row for when real measurement isn't available (first paint, a layout engine that
 * reports 0-width elements, or no strip width yet).
 */
function fallbackLayout(list: Slot[], edgeOrdered: (r: ComposerRow) => ComposerRow): { rows: ComposerRow[]; statsInline: boolean } {
    const left = list.filter((s) => s.side === "left").map((s) => s.key);
    const right = list.filter((s) => s.side === "right").map((s) => s.key);
    // statsInline true in the fallback — matches the pre-split
    // rendering every environment without measurement (JSDOM,
    // first paint) always had.
    if (left.length === 0 && right.length > 0) {
        return { rows: [edgeOrdered({ left: [right[0]], right: right.slice(1) })], statsInline: true };
    }
    return { rows: [edgeOrdered({ left, right })], statsInline: true };
}

/** The slots that take room: all but those the strip's own rules hide (0 px; Shell is never one of them). */
function visibleSlots(list: Slot[], widths: Record<string, number>): Slot[] {
    return list.filter((s) => s.key === "hostShell" || (widths[s.key] ?? 0) > 0);
}

/**
 * The rows for these slots at these widths. The CPU and memory readout rides with the context reading, as one
 * unit that is never split across lines (the readout, then the reading); the rest is `computeComposerRows`:
 * runtime/Shell anchored on the last row, account/context pinned left/right (Rev 9, see
 * docs/specs/SPEC_COMPOSER_STRIP_AUTH_COMPACT_SIDE_STABILITY_2026_09_16.md).
 */
function buildRows(visible: Slot[], widths: Record<string, number>, width: number, gapPx: number): ComposerRow[] {
    const beside = visible.some((s) => s.key === "stats") && visible.some((s) => s.key === "ctx");
    const units = visible
        .filter((s) => !(beside && s.key === "stats"))
        .map((s) => ({ key: s.key, width: (widths[s.key] ?? 0) + (beside && s.key === "ctx" ? (widths.stats ?? 0) + gapPx : 0) }));
    const rows = computeComposerRows(units, "hostShell", width, gapPx, "runtime", [{ leftKey: "auth", rightKey: "ctx" }]);
    if (!beside) return rows;
    const expand = (keys: string[]) => keys.flatMap((k) => (k === "ctx" ? ["stats", "ctx"] : [k]));
    return rows.map((r) => ({ left: expand(r.left), right: expand(r.right) }));
}

/** What these slots' layout costs at these widths: its rows, and a half more for each that overflows (what the forms are chosen by). */
function rowCountFor(list: Slot[], widths: Record<string, number>, width: number, gapPx: number): number {
    return rowsCost(buildRows(visibleSlots(list, widths), widths, width, gapPx), widths, gapPx, width);
}

function measuredLayout(
    list: Slot[],
    widths: Record<string, number>,
    width: number,
    statsWidth: Accessor<number>,
    refs: StripRefs,
    edgeOrdered: (r: ComposerRow) => ComposerRow,
): { rows: ComposerRow[]; statsInline: boolean } {
    // The real column-gap `.agent-composer-strip-row` applies between
    // its own left/right (and, single-row, stats) children — reading
    // it (not a hardcoded constant, which would silently drift if the
    // SCSS value ever changes) matches reagent P2 on PR #2808's same
    // rationale for the per-slot measurement effect above. (muxreview
    // P2, PR #2812: this used to read `--space-1-5`, the row's OWN
    // internal `-row-right` gap, not `--space-2`, the actual
    // `column-gap` between the row's left/right/stats children.)
    //
    // `* zoomRatio()` — regression found post-merge: `getComputedStyle`
    // returns this in LOCAL/unzoomed pixels, but `width`/`widths`
    // below are all `getBoundingClientRect()` (zoomed) values. Without
    // the correction, `gapPx` was too LARGE relative to everything
    // else under any non-1 ancestor zoom, overestimating how much
    // space is needed — the widest tier wrongly split into 2 lines,
    // and the per-pair capacity check (§3.2 step 6) rejected more
    // pairs than necessary as the pane narrowed, reproducing the very
    // one-sided-lines pattern this file exists to prevent. See
    // `zoomRatio`'s own doc comment above.
    const gapPx = stripGap(refs);

    // Codex P1, PR #2812: a slot hidden via this file's own SCSS
    // shed-content queries (e.g. `.agent-composer-strip-auth`
    // collapsing to
    // `display:none` below a container-width threshold) measures a
    // real 0px — correct, not a measurement bug. Feeding it into
    // `computeComposerRows` anyway let it consume a pairing partner
    // as if it were real content: a row could come back with both
    // `left`/`right` non-empty while one side renders nothing at all,
    // the exact visual bug this revision exists to prevent, just
    // relabeled as a "shed slot" instead of a wrapping zone. Excluded
    // from the pairing math below — never from rendering, since it
    // still needs a stable DOM home to stay measurable and pick back
    // up automatically once the container widens past its own
    // threshold again — then folded back into the last row afterward
    // via its own fallback `side`, where its zero width can never
    // visually unbalance anything.
    const visible = visibleSlots(list, widths);
    const shed = list.filter((s) => s.key !== "hostShell" && (widths[s.key] ?? 0) === 0);

    const built = buildRows(visible, widths, width, gapPx);

    // Stats share the single row's line only when slots PLUS stats
    // (plus the stats' own gap) genuinely fit — the check Codex P1
    // (PR #2812) originally put inside computeComposerRows, now the
    // answer to a different question: not "how do slots split" but
    // "where do the stats go." On failure the stats move to their
    // own line above the rows (the multi mount position) while the slots stay
    // exactly where the slot-only decision put them. See
    // `computeStatsInline`'s own doc comment.
    const slotsTotal = visible.reduce((sum, s) => sum + (widths[s.key] ?? 0), 0) + Math.max(0, visible.length - 1) * gapPx;
    const statsInline = computeStatsInline(built.length, slotsTotal, statsWidth(), gapPx, width);

    if (shed.length === 0) return { rows: built.map(edgeOrdered), statsInline };
    const target = built[built.length - 1] ?? { left: [], right: [] };
    for (const s of shed) {
        if (s.side === "left") target.left = [...target.left, s.key];
        else target.right = [...target.right, s.key];
    }
    return {
        rows: (built.length === 0 ? [target] : [...built.slice(0, -1), target]).map(edgeOrdered),
        statsInline,
    };
}

// Rev 7 — replaces `zones()`/`leftKeys()`/`rightKeys()`. Builds an
// explicit list of ROWS (see `computeComposerRows`'s own doc comment
// and docs/specs/SPEC_COMPOSER_STRIP_ROW_BASED_LAYOUT_2026_08_26.md)
// instead of two independently-wrapping zones, so every rendered line
// has content on both sides — the actual requirement six prior
// revisions all missed (docs/retro/retro-composer-strip-one-sided-lines-misdiagnosis-2026-08-26.md).
//
// Falls back to a SINGLE row via the fixed `side` field (Rev 5/6's
// own fallback, unchanged) whenever real measurement isn't available:
// first paint (before the effect above has run once), any layout
// engine that reports 0-width elements (this file's own JSDOM-based
// unit tests), or the ResizeObserver above hasn't delivered its first
// callback yet (`stripWidth() === 0`) — never an arbitrary/empty
// split, and never a spurious multi-row flash before the real width
// is known.
// Returns BOTH the row list and `statsInline` — whether the stats
// zone shares the single row's line (true only when slots-plus-stats
// genuinely fit together). Separated from row membership
// (user-reported regression, 2026-08-26): feeding `statsWidth` into
// `computeComposerRows`'s fit check made a too-wide stats zone split
// the SLOTS — the strip jumped from 1 visual line straight to 3
// (2 slot rows + the stats' own dedicated line), skipping the
// strictly-better middle tier this decision now creates: slots stay
// on one line and only the stats move to their own line (2 lines).
function createLayout(
    slots: Accessor<Slot[]>,
    slotWidths: Accessor<Record<string, number>>,
    stripWidth: Accessor<number>,
    statsWidth: Accessor<number>,
    refs: StripRefs,
): Accessor<{ rows: ComposerRow[]; statsInline: boolean }> {
    return createMemo((): { rows: ComposerRow[]; statsInline: boolean } => {
        const list = slots();
        const widths = slotWidths();
        const allMeasured = list.every((s) => widths[s.key] !== undefined);
        const totalMeasured = Object.values(widths).reduce((sum, w) => sum + w, 0);
        const width = stripWidth();

        // Edge priority (see `orderKeysForEdgePriority`) — applied to
        // EVERY return path below, fallback included, as the final step:
        // pure reordering within a side, after all row-membership and
        // shed decisions are made, so it can't interact with any of them.
        const edgeOrdered = edgeOrderer(list);

        if (!allMeasured || totalMeasured === 0 || width === 0) return fallbackLayout(list, edgeOrdered);
        return measuredLayout(list, widths, width, statsWidth, refs, edgeOrdered);
    });
}

function createSlotRenderer(
    slotByKey: Accessor<Map<string, Slot>>,
    refs: StripRefs,
    compactKeys: Accessor<Set<string>>,
    budgets: Accessor<Record<string, number>>
): (key: string, rowSide: Side) => JSX.Element {
    const renderSlot = (key: string, rowSide: Side) => {
        // untrack: see `slotByKey`'s own doc comment above — without
        // this, a bare `{slotByKey().get(key)?.render(...)}` would read
        // the reactive `slotByKey()` memo and re-invoke `.render()` (so
        // reconstructing e.g. AgentRuntimeDropup, resetting its own
        // open/selectedOptIndex signals) on every `slots()` recompute,
        // defeating the point of keying by string at all.
        const rendered = untrack(() => slotByKey().get(key)?.render(rowSide, () => compactKeys().has(key), () => budgets()[key]));
        return (
            // `display: contents` (agent-composer-strip-slot-measure,
            // _composer-strip.scss) — invisible to layout, so this
            // wrapper changes nothing about how the slot's own children
            // flex/wrap; it exists only to give the measurement effect
            // above a stable per-slot anchor.
            <span class="agent-composer-strip-slot-measure" ref={(el) => (refs.measureRefs[key] = el)}>
                {rendered}
            </span>
        );
    };
    return renderSlot;
}

interface StripViewParts {
    props: AgentComposerStripProps;
    refs: StripRefs;
    rows: () => ComposerRow[];
    statsInline: () => boolean;
    statsZone: (variant: "single" | "multi") => JSX.Element;
    precedingRowIndices: Accessor<number[]>;
    lastRowIndex: Accessor<number>;
    renderSlot: (key: string, rowSide: Side) => JSX.Element;
    probes: () => JSX.Element;
}

function StripView(view: StripViewParts): JSX.Element {
    const { props, refs, rows, statsInline, statsZone, precedingRowIndices, lastRowIndex, renderSlot, probes } = view;
    return (
        <div
            class="agent-composer-strip"
            classList={{ "agent-composer-strip--expanded": props.logOpen }}
            ref={(el) => { refs.stripRef = el; }}
        >
            {/* Rev 7 — one <div class="agent-composer-strip-row"> per entry
                in `rows()`, each with its own left- and right-anchored
                span, so every rendered line has content on both sides
                (see `computeComposerRows`'s own doc comment and
                docs/specs/SPEC_COMPOSER_STRIP_ROW_BASED_LAYOUT_2026_08_26.md).
                Replaces the old fixed `-controls`/`-right` zone spans,
                which independently decided when to wrap onto their own
                dedicated one-sided lines — the actual bug this revision
                answers (docs/retro/retro-composer-strip-one-sided-lines-misdiagnosis-2026-08-26.md). */}
            {/* Evicted stats render ABOVE the rows (user-directed,
                2026-08-26): as the pane narrows, the centered token
                stats float UP off the shared line, keeping the
                interactive slot rows anchored at the bottom next to the
                composer instead of being pushed down by a stats line. */}
            <Show when={!statsInline()}>{statsZone("multi")}</Show>

            <div class="agent-composer-strip-rows" ref={(el) => { refs.rowsRef = el; }}>
                <For each={precedingRowIndices()}>
                    {(rowIndex) => (
                        <div class="agent-composer-strip-row">
                            <span class="agent-composer-strip-row-left">
                                <For each={rows()[rowIndex]?.left ?? []}>{(key) => renderSlot(key, "left")}</For>
                            </span>
                            <span class="agent-composer-strip-row-right">
                                <For each={rows()[rowIndex]?.right ?? []}>{(key) => renderSlot(key, "right")}</For>
                            </span>
                        </div>
                    )}
                </For>
                {/* The anchors' row, rendered from its own fixed element
                    rather than as the `<For>`'s last entry — see
                    `precedingRowIndices`'s doc comment (Codex P2, PR
                    #2839). This is the row `runtime`/`hostShell` always
                    occupy (the sole row when single-row, the bottom row
                    when multi-row), so keeping it one stable subtree is
                    what preserves `AgentRuntimeDropup`'s open state across
                    a resize that changes the row count. The inline stats
                    zone lives here too: `statsInline()` is only ever true
                    at one row, and at one row this IS that row. */}
                <Show when={rows().length > 0}>
                    <div class="agent-composer-strip-row">
                        <span class="agent-composer-strip-row-left">
                            <For each={rows()[lastRowIndex()]?.left ?? []}>{(key) => renderSlot(key, "left")}</For>
                        </span>
                        <Show when={statsInline()}>{statsZone("single")}</Show>
                        <span class="agent-composer-strip-row-right">
                            <For each={rows()[lastRowIndex()]?.right ?? []}>{(key) => renderSlot(key, "right")}</For>
                        </span>
                    </div>
                </Show>
            </div>
            {probes()}
        </div>
    );
}

// ── Component ──────────────────────────────────────────────────────────────

function createSlots(props: AgentComposerStripProps, d: StripDerived): Accessor<Slot[]> {
    // Dynamic left/right pooling — see the file-header comment for why a
    // static per-item zone assignment left the controls zone empty for
    // non-Claude/no-process/unknown-auth panes. Each slot's `render`
    // reproduces exactly what that element rendered under its old fixed
    // zone (same classes/handlers/titles) — only WHICH zone renders it,
    // computed fresh here every time, changed. `side` is a fixed semantic
    // preference (left = state/config, right = counters + the one real
    // action) — see `createLayout` for how the "left must never be
    // completely empty" override works, and
    // docs/specs/SPEC_COMPOSER_STRIP_DYNAMIC_BALANCE_2026_08_24.md Rev 4
    // for why this replaced two earlier attempts at balancing by a
    // computed integer "weight" (count-based, then weight-based subset
    // partition) — both were solving the wrong layer: the actual dead-
    // space bug was the CSS forcing both zones to equal width regardless
    // of content (see _composer-strip.scss), not which slot goes where.
    // `interactive` — feeds the edge-priority ordering (see
    // `orderKeysForEdgePriority`): true for slots whose primary content
    // is something you CLICK (buttons/dropdowns), false for passive
    // status/info. `render` receives the side of the row it is being
    // mounted into, so composite slots (ctx, hostShell) can mirror their
    // own internal order to keep their interactive element on the outer
    // end — the side is fixed for the lifetime of one render() call
    // (a slot changing sides leaves one <For> and enters the other,
    // re-invoking render with the new side), so it is deliberately a
    // plain parameter, not a reactive read.
    // Whether the readout has figures yet: the memo changes once, so the slots are not rebuilt by its ticks.
    const stats = props.blockId ? useBlockStats(props.blockId) : () => null;
    const hasStats = createMemo(() => stats() != null);
    const slots = createMemo((): Slot[] => {
        const out: Slot[] = [];

        if (d.showControls()) out.push(runtimeSlot(props));

        if (props.authStatus === "authenticated" || props.authStatus === "unauthenticated" || accountKnown(props)) out.push(authSlot(props));

        // A pane whose provider reports no context reading (codex, gemini, …)
        // still gets the popover — cost and processed tokens — once a turn
        // has reported totals.
        if (d.ctxText() != null || d.hasSessionToManage() || props.sessionTotals != null) out.push(ctxSlot(props, d));

        // The CPU and memory readout, after the context reading (the last of the forms to give way).
        if (hasStats()) out.push(statsSlot(props));

        // The pane's view (agent-view-controls.ts viewChipLabel), only while it is not the plain Full.
        // The memo reads only whether there is a chip; the text is read in render, so a change of preset does not
        // rebuild the slots (and close the runtime panel).
        if (props.viewChip && props.onOpenView) out.push(viewSlot(props));

        // Always present (Shell has no visibility gate) and always last
        // in pool order — see `createLayout` for why that keeps it on the
        // right except in the fully degenerate one-slot-total case.
        out.push(hostShellSlot(props));

        return out;
    });
    return slots;
}

export const AgentComposerStrip = (props: AgentComposerStripProps): JSX.Element => {
    const d = createStripDerived(props);

    const slots = createSlots(props, d);

    const refs: StripRefs = { stripRef: undefined, rowsRef: undefined, measureRefs: {} };
    const stripWidth = useStripWidth(refs);
    const slotWidths = useSlotWidths(slots, stripWidth, refs);
    // Which slots show their compact form, from hidden probes of both forms (composer-forms-probes.tsx).
    const forms = useSlotForms(slots, stripWidth, slotWidths, (widths) => rowCountFor(slots(), widths, stripWidth(), stripGap(refs)));
    // The stats zone and its measured footprint (composer-stats-zone.tsx).
    const { statsWidth, statsZone } = useStatsZone(stripWidth, slotWidths);
    const slotByKey = createSlotMap(slots);
    const layout = createLayout(slots, forms.layoutWidths, stripWidth, statsWidth, refs);

    const rows = (): ComposerRow[] => layout().rows;
    const statsInline = (): boolean => layout().statsInline;

    // Row COUNT rarely changes across recomputes — plain number primitives
    // compare by value, so `<For>` over indices preserves each row's own
    // subtree (and everything under it) across recomputes that don't
    // change how many rows exist. Iterating `rows()` objects directly
    // would repeat the exact reagent-P1 identity bug the slot-level fix
    // above already fixed, one level up.
    //
    // The LAST row is deliberately NOT part of this list — it renders from
    // its own fixed element below (Codex P2, PR #2839). `<For>` over
    // indices only preserves a subtree while the row COUNT is unchanged:
    // crossing 1 row -> 2 rows makes index 1 a brand-new subtree, so any
    // slot moving into it is destroyed and rebuilt. That is exactly what
    // the Rev 8 anchor constraint forces on `runtime`/`hostShell` (they're
    // in the sole row at index 0 when single-row, the last row at index
    // N-1 when multi-row), and `AgentRuntimeDropup` keeps its open/
    // selectedOptIndex state in component-local signals — so resizing a
    // pane across that boundary with the model menu open silently closed
    // it and lost keyboard selection. Verified live over CDP: pre-change
    // the trigger node survived the crossing, with the anchor constraint
    // it did not. Hoisting the anchors' row out of the `<For>` gives them
    // one stable home for every row count, which is the DOM expression of
    // the same "these two never travel" requirement the constraint states
    // at the layout level.
    const precedingRowIndices = createMemo(() => rows().slice(0, -1).map((_, i) => i));
    const lastRowIndex = createMemo(() => rows().length - 1);

    const renderSlot = createSlotRenderer(slotByKey, refs, forms.compactKeys, forms.budgets);

    return (
        <StripView props={props} refs={refs} rows={rows} statsInline={statsInline} statsZone={statsZone} precedingRowIndices={precedingRowIndices} lastRowIndex={lastRowIndex} renderSlot={renderSlot} probes={forms.probes} />
    );
};

AgentComposerStrip.displayName = "AgentComposerStrip";

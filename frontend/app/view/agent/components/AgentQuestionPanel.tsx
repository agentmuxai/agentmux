// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentQuestionPanel — surfaced when a `ToolNode` in the pane has
 * `status === "awaiting_answer"` (the agent called the `AskUserQuestion`
 * tool and is blocked on the user's answer). Renders the question(s) with
 * single- or multi-select options plus a free-text "Other", and submits the
 * answer so the caller can deliver it back to the agent as a tool_result.
 *
 * Also runs an auto-timeout (default 30s, user-configurable via
 * `agent:askquestiontimeoutms`) so an unanswered question can never block
 * the agent's turn forever: any question the user hasn't touched by zero is
 * filled in with its recommended option and the (possibly-merged) answer is
 * submitted automatically. See §2.3 for why this merges rather than
 * disarming on first interaction. Hovering the panel — or, as of the
 * keyboard-pause spec below, pressing any key while focus is inside the
 * panel (Tab-navigating options, typing into "Other") — hides the countdown
 * and pauses the underlying deadline for a flat 15s from that trigger, then
 * unconditionally resumes at a fresh timeout regardless of whether the mouse
 * or keyboard is still active — a bounded, self-resuming pause, not the
 * permanent disarm §2.3/§5.1 rejected. Both triggers share one mechanism —
 * see the keyboard-pause spec below.
 *
 * Spec: docs/specs/SPEC_ASK_USER_QUESTION_2026_06_15.md,
 * docs/specs/SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md,
 * docs/specs/SPEC_ASK_USER_QUESTION_TIMEOUT_HOVER_PAUSE_2026_08_10.md,
 * docs/specs/SPEC_ASK_USER_QUESTION_TIMEOUT_KEYBOARD_PAUSE_2026_08_20.md.
 */

import { createEffect, createMemo, createSignal, For, onCleanup, Show, untrack, type Accessor, type JSX } from "solid-js";
import { usePaneOverlay } from "@/app/platform/pane-overlay";
import { showTextInputContextMenu } from "@/app/store/contextmenu";
import { getSettingsKeyAtom } from "@/app/store/global";
import type { AskUserQuestionAnswer, AskUserQuestionOption, AskUserQuestionRequest, ToolNode } from "../types";
import "./AgentQuestionPanel.scss";

/** Fallback when `agent:askquestiontimeoutms` is unset — see
 *  `autoTimeoutMs()` inside the component. Was a hardcoded constant per
 *  SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md §5.2 ("not user-
 *  configurable in v1... a reasonable follow-up if requested later"); now
 *  user-configurable, this is only the default. */
const DEFAULT_AUTO_TIMEOUT_MS = 30_000;

/** How long the countdown stays hidden after a hover into the panel, before
 *  an unanswered question-set's timer resumes (fresh autoTimeoutMs(), not
 *  resumed from wherever it was paused) — a flat window timed from the
 *  triggering `mouseenter`, unconditional regardless of whether the mouse is
 *  still over the panel when it elapses. See
 *  SPEC_ASK_USER_QUESTION_TIMEOUT_HOVER_PAUSE_2026_08_10.md §3. */
const HOVER_HIDE_GRACE_MS = 15_000;

/** Matches a Claude Code AskUserQuestion "(Recommended)" label suffix,
 *  case-insensitively, with optional trailing whitespace. */
const RECOMMENDED_RE = /\(recommended\)\s*$/i;

/**
 * The option(s) to auto-select for a question at timeout. There is no
 * explicit "recommended" field in the wire schema — only Claude Code's own
 * convention for this tool: mark the recommended option's label with a
 * trailing "(Recommended)", and make it the first option in the list. When
 * no label is flagged, falling back to the first option is always safe —
 * worst case it's just "the first option," the same outcome as a human
 * clicking through without reading closely.
 */
export function recommendedOptions(options: AskUserQuestionOption[]): AskUserQuestionOption[] {
    const flagged = options.filter((o) => RECOMMENDED_RE.test(o.label));
    if (flagged.length > 0) return flagged;
    return options.length > 0 ? [options[0]] : [];
}

export interface AnswerOutcome {
    tool_use_id: string;
    answers: AskUserQuestionAnswer[];
    /** Control-protocol `updatedInput.answers`: each question's TEXT → the
     *  chosen label (string), a label array (multiSelect), or free-text ("Other").
     *  This is what the agent CLI consumes via the control_response. Spec:
     *  SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md §2.3. */
    answers_map: Record<string, string | string[]>;
    /** Flat-text rendering kept for the optimistic node summary + the one-shot/
     *  container follow-up fallback (which has no control channel). */
    answer_text: string;
    /** How many of `answers` were filled in by the 30s auto-timeout rather
     *  than the user, per SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md
     *  §2.5 — `0` for a fully manual submit, up to `answers.length` for a
     *  fully timed-out one. A plain boolean would conflate "user answered
     *  some, timeout filled the rest" with "user never touched anything,"
     *  which matters for anyone auditing the transcript later. */
    autoFilledCount: number;
}

interface AgentQuestionPanelProps {
    /** Pending questions, oldest first. The panel shows the head. */
    pending: Accessor<ToolNode[]>;
    /** User answer. Caller advances the queue by transitioning the node. */
    onAnswer: (outcome: AnswerOutcome) => void | Promise<void>;
    /** Defer — leave the node in awaiting_answer; minimise without answering. */
    onDefer?: () => void;
}

/** Per-question working state. */
interface QState {
    selected: string[];
    other: string;
}

// Mirrors AgentDecisionPanel's clip: register the pane overlay against the
// live panel root so it floats above the conversation. Mounted only while the
// panel is visible so usePaneOverlay sees an attached element.
const QuestionPanelClip = (p: { getEl: Accessor<HTMLElement | null | undefined> }): JSX.Element => {
    usePaneOverlay(p.getEl);
    return null;
};

export const AgentQuestionPanel = (props: AgentQuestionPanelProps): JSX.Element => {
    let rootRef: HTMLElement | undefined;

    const head = createMemo<ToolNode | null>(() => props.pending()[0] ?? null);
    const queueDepth = () => props.pending().length;
    const request = (): AskUserQuestionRequest | null => head()?.question ?? null;

    /** `agent:askquestiontimeoutms` if set to a positive number, else
     *  `DEFAULT_AUTO_TIMEOUT_MS`. Read fresh at every re-arm point below
     *  (not cached) so a mid-session settings change takes effect on the
     *  next question rather than requiring a reload.
     *
     *  `untrack`ed deliberately: every call site below lives inside a
     *  `createEffect` keyed on `tool_use_id`/`hidden()`, not on this
     *  setting. Without `untrack`, Solid registers the settings read as a
     *  dependency of whichever enclosing effect calls this — so the
     *  question-reset effect (which unconditionally wipes `state`/
     *  `minimized`/`hidden` — "keyed on tool_use_id so we never inherit a
     *  prior question's selections") would ALSO re-run and discard an
     *  in-progress answer whenever the user merely adjusted this setting
     *  in Settings -> Advanced, unrelated to any new question arriving.
     *  Confirmed independently by reagent (P1) and Codex (P2) on PR #2670.
     *  `untrack` here still reads the CURRENT value at each call — it only
     *  stops that read from being treated as a reactive trigger. */
    const autoTimeoutMs = (): number => {
        const v = untrack(() => getSettingsKeyAtom("agent:askquestiontimeoutms")());
        return typeof v === "number" && v > 0 ? v : DEFAULT_AUTO_TIMEOUT_MS;
    };

    const [minimized, setMinimized] = createSignal(false);
    const [state, setState] = createSignal<QState[]>([]);
    /** Milliseconds left before the auto-timeout fires. See the timer effect
     *  below (defined after `submit`, once all its dependencies exist). */
    const [remainingMs, setRemainingMs] = createSignal(autoTimeoutMs());
    /** True while the countdown is suppressed by a recent hover. Drives both
     *  the UI (rendered nothing while true, §3.4 of the hover-pause spec)
     *  and whether the timer effect below is armed. Bounded, NOT tied to
     *  "is the mouse currently over the panel" — see the doc comment on
     *  `onPanelPointerEnter` below for why. */
    const [hidden, setHidden] = createSignal(false);
    let hideTimeoutId: ReturnType<typeof setTimeout> | undefined;

    const clearHideTimer = () => {
        if (hideTimeoutId !== undefined) {
            clearTimeout(hideTimeoutId);
            hideTimeoutId = undefined;
        }
    };
    // The hide timer is a raw setTimeout, not owned by a reactive effect (it
    // must survive across hidden()/tool_use_id changes within its own
    // window) — so it needs its own top-level cleanup on unmount, same as
    // every interval elsewhere in this file gets via its owning effect.
    onCleanup(clearHideTimer);

    // Reset working state whenever the head question changes (new tool_use_id,
    // queue advance). Keyed on tool_use_id so we never inherit a prior
    // question's selections. Also resets hover/hidden state so a fresh
    // question-set never inherits a stale hover history from the previous
    // one (SPEC_ASK_USER_QUESTION_TIMEOUT_HOVER_PAUSE_2026_08_10.md §3.3).
    createEffect(() => {
        const r = request();
        void r?.tool_use_id; // touch so the effect re-runs on change
        setMinimized(false);
        setState((r?.questions ?? []).map(() => ({ selected: [], other: "" })));
        setRemainingMs(autoTimeoutMs());
        clearHideTimer();
        setHidden(false);
    });

    // Mouse enters the expanded panel. See
    // SPEC_ASK_USER_QUESTION_TIMEOUT_HOVER_PAUSE_2026_08_10.md §3.1 for why
    // this is scoped to the whole panel (not just the "Other" input) and
    // excludes the minimized chip entirely — wired only on the expanded
    // panel's root div below.
    //
    // Hides for a flat HOVER_HIDE_GRACE_MS from THIS entry, then
    // unconditionally resumes — deliberately NOT "stays hidden for as long
    // as the mouse remains over the panel." An earlier version of this
    // feature did that and a test caught the resulting regression:
    // `userEvent.click()` on any option inside the panel fires a real
    // `mouseenter` on the way to the click (the pointer has to be over the
    // target to click it — true in a real browser too, not just this test
    // harness), so answering by mouse and then stepping away with the
    // cursor left parked over the panel would have paused the timer
    // indefinitely, since nothing ever fires `mouseleave`. That's the exact
    // "work does not stop" failure §5.1 of the original timeout spec
    // rejected, reintroduced through hover instead of a permanent disarm on
    // click. Bounding every hide to a flat window, timed from entry and
    // independent of whether the mouse is still there, closes that gap: the
    // worst case is "paused for at most HOVER_HIDE_GRACE_MS," never
    // indefinite. A fresh `mouseenter` (the mouse actually leaving and
    // coming back) re-arms a new window — that's the "recursively" behavior
    // the feature asks for.
    const onPanelPointerEnter = () => {
        clearHideTimer();
        setHidden(true);
        hideTimeoutId = setTimeout(() => {
            hideTimeoutId = undefined;
            setHidden(false); // timer effect below re-arms at a fresh autoTimeoutMs()
        }, HOVER_HIDE_GRACE_MS);
    };

    const setQ = (i: number, next: Partial<QState>) => {
        setState((prev) => prev.map((q, idx) => (idx === i ? { ...q, ...next } : q)));
    };

    const toggleOption = (qi: number, label: string, multi: boolean) => {
        const cur = state()[qi];
        if (!cur) return;
        if (multi) {
            const has = cur.selected.includes(label);
            setQ(qi, { selected: has ? cur.selected.filter((l) => l !== label) : [...cur.selected, label] });
        } else {
            // Single-select: choosing an option clears any "Other" text.
            setQ(qi, { selected: [label], other: "" });
        }
    };

    const setOther = (qi: number, text: string, multi: boolean) => {
        // For single-select, typing "Other" supersedes the radio choice.
        if (!multi && text.length > 0) {
            setQ(qi, { other: text, selected: [] });
        } else {
            setQ(qi, { other: text });
        }
    };

    const questionAnswered = (qi: number): boolean => {
        const s = state()[qi];
        return !!s && (s.selected.length > 0 || s.other.trim().length > 0);
    };

    const allAnswered = createMemo<boolean>(() => {
        const r = request();
        if (!r) return false;
        return r.questions.every((_, i) => questionAnswered(i));
    });

    const buildOutcome = (autoFilledCount: number): AnswerOutcome | null => {
        const r = request();
        if (!r) return null;
        const answers: AskUserQuestionAnswer[] = r.questions.map((q, i) => {
            const s = state()[i] ?? { selected: [], other: "" };
            const other = s.other.trim();
            return { header: q.header, selected: s.selected, ...(other ? { other } : {}) };
        });
        const answer_text = answers
            .map((a) => {
                const parts = [...a.selected];
                if (a.other) parts.push(`Other: ${a.other}`);
                return `${a.header}: ${parts.join(", ")}`;
            })
            .join("\n");
        // Control-protocol answers map, keyed by each question's TEXT (not header).
        // Free-text "Other" wins; multiSelect → label array; else single label.
        const answers_map: Record<string, string | string[]> = {};
        r.questions.forEach((q, i) => {
            const s = state()[i] ?? { selected: [], other: "" };
            const other = s.other.trim();
            if (other) answers_map[q.question] = other;
            else if (q.multiSelect) answers_map[q.question] = s.selected;
            else answers_map[q.question] = s.selected[0] ?? "";
        });
        return { tool_use_id: r.tool_use_id, answers, answers_map, answer_text, autoFilledCount };
    };

    // `autoFilledCount` defaults to 0 — every manual call site (the Submit
    // button, Enter-to-submit) leaves it unset. Only the timeout path below
    // passes a non-zero count.
    const submit = (autoFilledCount = 0) => {
        if (!allAnswered()) return;
        const outcome = buildOutcome(autoFilledCount);
        if (outcome) void props.onAnswer(outcome);
    };

    // Fill in the recommended default for every question the user hasn't
    // touched yet (per `questionAnswered`); questions already answered —
    // fully or partially — are left exactly as-is. Returns how many
    // questions were filled, for the transcript's audit trail (§2.5 of the
    // spec). Called only from the timeout below, never on manual submit.
    //
    // GUARANTEES every question is answered by the time this returns — even
    // a malformed AskUserQuestion with a zero-length `options` array (so
    // `recommendedOptions` has nothing to select) falls back to a free-text
    // placeholder. This matters because the timer effect below clears its
    // interval unconditionally before calling `submit()`: if a question
    // came back from this function still unanswered, `submit()`'s
    // `allAnswered()` gate would silently no-op and — with the interval
    // already gone — the panel would be stuck forever with no further
    // timeout retry, defeating the whole "work never stalls" guarantee for
    // exactly the unattended-run case this feature exists to protect
    // (reagent P1, PR #2441).
    const applyRecommendedDefaults = (): number => {
        const r = request();
        if (!r) return 0;
        let count = 0;
        r.questions.forEach((q, i) => {
            if (questionAnswered(i)) return;
            count++;
            const recommended = recommendedOptions(q.options);
            if (recommended.length > 0) {
                setQ(i, { selected: recommended.map((o) => o.label), other: "" });
            } else {
                // No options at all to recommend — leave a free-text note
                // rather than an unanswerable blank, so `allAnswered()`
                // passes and the merged outcome can still submit.
                setQ(i, { selected: [], other: "No option was available to auto-select" });
            }
        });
        return count;
    };

    // 30s auto-timeout: fires unconditionally at zero once armed, regardless
    // of any *past* interaction. Deliberately NOT disarmed permanently on the
    // first click/keystroke — an earlier design did that, and it was
    // rejected because it directly undercuts the feature's own goal ("work
    // does not stop"): a user who answers one question in a multi-question
    // set and then steps away would otherwise cancel the safety net
    // entirely, leaving the rest blocked forever. Instead,
    // `applyRecommendedDefaults` merges — anything the user already answered
    // survives untouched. See
    // docs/specs/SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md §5.1.
    //
    // Gated on `hidden()` (SPEC_ASK_USER_QUESTION_TIMEOUT_HOVER_PAUSE_2026_08_10.md
    // §3.3): this is a *live, strictly bounded* pause, not a permanent
    // disarm — a hover starts a flat HOVER_HIDE_GRACE_MS window, and once it
    // elapses this effect re-runs and re-arms at a fresh autoTimeoutMs()
    // UNCONDITIONALLY, regardless of whether the mouse is still over the
    // panel (see onPanelPointerEnter's doc comment and spec §9 for why —
    // an earlier, rejected version of this stayed hidden for as long as the
    // mouse remained over the panel, which reopened §5.1's exact failure
    // mode via a click's own mouseenter). That distinction is why this
    // doesn't reopen §5.1's rejected design: the countdown can never be
    // suppressed for longer than one HOVER_HIDE_GRACE_MS window per hover.
    //
    // A separate effect (rather than folding into the reset effect above)
    // because it depends on `submit`/`applyRecommendedDefaults`, which in
    // turn depend on `setQ`/`questionAnswered`/`allAnswered` — keeping the
    // reset effect's own dependencies minimal and unchanged.
    createEffect(() => {
        const r = request();
        void r?.tool_use_id; // touch so the effect re-runs on change, same as the reset effect
        if (!r || hidden()) return; // paused while hidden; re-arms when hidden() flips false

        setRemainingMs(autoTimeoutMs()); // fresh retrigger, not resumed from wherever it was paused
        const intervalId = setInterval(() => {
            setRemainingMs((prev) => {
                if (prev <= 1000) {
                    clearInterval(intervalId);
                    submit(applyRecommendedDefaults());
                    return 0;
                }
                return prev - 1000;
            });
        }, 1000);
        onCleanup(() => clearInterval(intervalId));
    });

    const defer = () => {
        // Hover-pause is a full-panel-only affordance (the minimized chip
        // has no typing surface) — force an immediate resume so minimizing
        // always returns the timer to its original, hover-independent
        // behavior, preserving "keeps running while minimized" unchanged.
        // SPEC_ASK_USER_QUESTION_TIMEOUT_HOVER_PAUSE_2026_08_10.md §3.1, §3.3.
        clearHideTimer();
        setHidden(false);
        setMinimized(true);
        props.onDefer?.();
    };

    // Any `<input>`/`<textarea>`/contentEditable is "editable" — this is the
    // broad check (reagent P1, PR #2060: an earlier version of this file
    // narrowed it to TEXTAREA/contentEditable only, so it no longer
    // recognized a plain text `<input>` elsewhere in the pane — e.g. the
    // Ctrl+F search bar, AgentSearchBar.tsx — as something Enter shouldn't
    // be stolen from, silently submitting a fully-answered pending question
    // while the user was just navigating search matches).
    const isEditableTarget = (target: EventTarget | null): boolean => {
        const el = target as HTMLElement | null;
        if (!el) return false;
        if (el.tagName === "INPUT" || el.tagName === "TEXTAREA") return true;
        return el.isContentEditable;
    };

    // Shared gate for both keyboard-pause trigger paths below (`handleKey`'s
    // keydown and `handleFocusIn`'s focusin). A target counts as
    // pause-worthy engagement only if it's actually inside this panel's own
    // DOM AND the panel is in its expanded (non-minimized) state AND we're
    // not already paused:
    //  - `!minimized()`: while minimized, `rootRef` is reassigned to the
    //    separate `<button class="agent-question-panel-minimized">` (below),
    //    so a keydown/Tab landing on that focusable button would otherwise
    //    read as "inside the panel" and incorrectly pause a countdown that's
    //    supposed to keep running unaffected while minimized — codex P2,
    //    PR #2787.
    //  - `!hidden()`: only the *transition into* the paused state re-arms
    //    the flat HOVER_HIDE_GRACE_MS window. Unlike `mouseenter` — which a
    //    real browser only fires on an actual boundary-crossing, so it can't
    //    be spammed by normal use — `keydown` fires on every keystroke,
    //    including OS key-repeat while a key is held and every character of
    //    continuous typing. Re-arming unconditionally on every one of those
    //    (the original implementation) meant typing faster than 15s apart,
    //    or simply holding a key, suppressed the auto-timeout indefinitely —
    //    reagentx P1, PR #2787, breaking the "paused for at most one
    //    HOVER_HIDE_GRACE_MS window" invariant documented on the timer
    //    effect above. Gating here bounds every pause to exactly one window
    //    regardless of how many qualifying events fire inside it; a fresh
    //    trigger only re-arms once that window has actually elapsed and
    //    `hidden()` has flipped back to false.
    const maybePauseFor = (target: EventTarget | null) => {
        const inPanel = !!rootRef && !!target && rootRef.contains(target as Node);
        if (inPanel && !minimized() && !hidden()) onPanelPointerEnter();
    };

    const handleKey = (e: KeyboardEvent) => {
        const target = e.target as HTMLElement | null;
        // Scope to this panel's own pane so a question in pane A doesn't
        // react to keystrokes typed in pane B. Mirrors AgentDecisionPanel
        // (codex P1, PR #556).
        const paneRoot = rootRef?.closest(".agent-view") as HTMLElement | null;
        if (paneRoot && target && !paneRoot.contains(target)) return;

        // Whether the keystroke actually originated inside this panel's own
        // DOM (an option, the "Other" input, or the panel root itself) —
        // mirrors AgentDecisionPanel's `inPanel` (AgentDecisionPanel.tsx:208).
        const inPanel = !!rootRef && !!target && rootRef.contains(target);

        // Any keydown that lands inside this panel counts as engagement,
        // the same as a mouseenter — reuses the exact same pause mechanism
        // (hide the countdown, resume unconditionally after a flat
        // HOVER_HIDE_GRACE_MS) rather than a parallel one, so a user
        // answering entirely by keyboard (Tab between options, typing into
        // "Other") gets the same breathing room a mouse-hovering user
        // already does. Scoped to `inPanel`, NOT the broader `paneRoot`
        // scope Escape uses below — a keystroke elsewhere in this pane
        // (the chat composer, Ctrl+F) isn't engagement with this question.
        // Deliberately unconditional on which key, including Enter/Escape:
        // both already tear down or reset this same pause state via their
        // own existing paths immediately below/in `defer()`, so firing this
        // first for them is a harmless, immediately-superseded no-op, not
        // worth special-casing out. See
        // SPEC_ASK_USER_QUESTION_TIMEOUT_KEYBOARD_PAUSE_2026_08_20.md §2,
        // and §8 for the `maybePauseFor` gating added after review.
        maybePauseFor(target);

        if (e.key === "Enter" && !e.shiftKey) {
            // Outside the panel, don't hijack Enter from a real editable
            // control elsewhere in the pane (composer textarea, Ctrl+F
            // search input, etc.). Inside the panel, every control (options,
            // "Other" free-text input) submits on Enter regardless — none of
            // them treat Enter as "insert a newline".
            if (!inPanel && isEditableTarget(target)) return;
            e.preventDefault();
            submit();
        } else if (e.key === "Escape") {
            e.preventDefault();
            defer();
        }
    };

    // Tab moving focus INTO the panel from outside it fires its `keydown`
    // with `e.target` still the element that's *about to lose* focus —
    // browsers move focus only after the keydown's default action runs — so
    // `handleKey`'s `inPanel` check above misses exactly the keystroke that
    // causes a keyboard-only user's first entry into the panel via Tab.
    // `focusin` bubbles and fires once focus has actually landed inside the
    // panel, so listening for it separately (reusing the same
    // `maybePauseFor` gate) catches that case without complicating
    // `handleKey`'s own target-at-dispatch-time logic — codex P2, PR #2787.
    const handleFocusIn = (e: FocusEvent) => maybePauseFor(e.target);

    // Global capture-phase listener, mirroring AgentDecisionPanel: the panel
    // has tabindex=-1 and never auto-focuses, so a plain onKeyDown on the
    // root div only fired once the user had already clicked something
    // inside it — Enter otherwise never reached the handler at all.
    createEffect(() => {
        if (!request()) return;
        const onWindowKey = (e: KeyboardEvent) => handleKey(e);
        window.addEventListener("keydown", onWindowKey, true);
        // `focusin` already bubbles to `window`, so no capture flag needed.
        window.addEventListener("focusin", handleFocusIn);
        onCleanup(() => {
            window.removeEventListener("keydown", onWindowKey, true);
            window.removeEventListener("focusin", handleFocusIn);
        });
    });

    const countdownSeconds = () => Math.ceil(remainingMs() / 1000);
    /** Color-escalation band for the countdown chip. Thresholds/tokens per
     *  SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md §2.4/§5.3. */
    const countdownSeverity = (): "default" | "warning" | "critical" => {
        const s = countdownSeconds();
        if (s <= 5) return "critical";
        if (s <= 10) return "warning";
        return "default";
    };

    return (
        <Show when={request()} keyed>
            {(r) => (
                <Show
                    when={!minimized()}
                    fallback={
                        <button
                            type="button"
                            ref={(el) => (rootRef = el)}
                            class="agent-question-panel-minimized"
                            onClick={() => setMinimized(false)}
                        >
                            <span class="agent-question-panel-icon" aria-hidden="true">❓</span>
                            <span>Question waiting</span>
                            <span
                                class="agent-question-panel-countdown"
                                classList={{
                                    "agent-question-panel-countdown--warning": countdownSeverity() === "warning",
                                    "agent-question-panel-countdown--critical": countdownSeverity() === "critical",
                                }}
                            >
                                auto-selects in {countdownSeconds()}s
                            </span>
                            <span class="agent-question-panel-minimized-cta">click to answer</span>
                        </button>
                    }
                >
                    <QuestionPanelClip getEl={() => rootRef} />
                    {/* eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions */}
                    <div
                        ref={(el) => (rootRef = el)}
                        class="agent-question-panel"
                        role="group"
                        aria-label="Agent question"
                        tabindex={-1}
                        onMouseEnter={onPanelPointerEnter}
                    >
                        <div class="agent-question-panel-header">
                            <span class="agent-question-panel-icon" aria-hidden="true">❓</span>
                            <span class="agent-question-panel-title">The agent is asking</span>
                            <Show when={queueDepth() > 1}>
                                <span class="agent-question-panel-queue">+{queueDepth() - 1} more</span>
                            </Show>
                            {/* Rendered nothing (not just visually hidden) while hidden() —
                                SPEC_ASK_USER_QUESTION_TIMEOUT_HOVER_PAUSE_2026_08_10.md §3.4. */}
                            <Show when={!hidden()}>
                                <span
                                    class="agent-question-panel-countdown"
                                    classList={{
                                        "agent-question-panel-countdown--warning": countdownSeverity() === "warning",
                                        "agent-question-panel-countdown--critical": countdownSeverity() === "critical",
                                    }}
                                >
                                    Auto-selects recommended in {countdownSeconds()}s
                                </span>
                            </Show>
                        </div>

                        {/* Scrollable middle region — header (above) and actions
                            (below) stay pinned via position: sticky so the
                            countdown and Submit/Answer-later buttons are never
                            scrolled out of reach on a long question set.
                            Spec: docs/specs/SPEC_ASK_USER_QUESTION_PANEL_SCROLL_2026_08_25.md. */}
                        <div class="agent-question-panel-scroll">
                            <For each={r.questions}>
                                {(q, qi) => (
                                    <fieldset class="agent-question-panel-q">
                                        <legend class="agent-question-panel-q-prompt">
                                            <span class="agent-question-panel-q-chip">{q.header}</span>
                                            {q.question}
                                        </legend>
                                        <div class="agent-question-panel-options">
                                            <For each={q.options}>
                                                {(opt) => {
                                                    const checked = () =>
                                                        state()[qi()]?.selected.includes(opt.label) ?? false;
                                                    // Highlight which option(s) the 30s auto-timeout would pick,
                                                    // so a watching user can predict the outcome before it
                                                    // happens. Display-neutral: the label text itself (including
                                                    // any "(Recommended)" suffix) is unchanged.
                                                    const recommended = () =>
                                                        recommendedOptions(q.options).some((o) => o.label === opt.label);
                                                    return (
                                                        <label
                                                            class="agent-question-panel-option"
                                                            classList={{
                                                                "agent-question-panel-option--checked": checked(),
                                                                "agent-question-panel-option--recommended": recommended(),
                                                            }}
                                                        >
                                                            <input
                                                                type={q.multiSelect ? "checkbox" : "radio"}
                                                                name={`amux-q-${r.tool_use_id}-${qi()}`}
                                                                checked={checked()}
                                                                onChange={() => toggleOption(qi(), opt.label, q.multiSelect)}
                                                            />
                                                            <span class="agent-question-panel-option-body">
                                                                <span class="agent-question-panel-option-label">{opt.label}</span>
                                                                <Show when={opt.description}>
                                                                    <span class="agent-question-panel-option-desc">{opt.description}</span>
                                                                </Show>
                                                            </span>
                                                        </label>
                                                    );
                                                }}
                                            </For>
                                            <label class="agent-question-panel-other">
                                                <span class="agent-question-panel-other-label">Other</span>
                                                <input
                                                    type="text"
                                                    class="agent-question-panel-other-input"
                                                    placeholder="Type a custom answer…"
                                                    value={state()[qi()]?.other ?? ""}
                                                    onInput={(e) => setOther(qi(), e.currentTarget.value, q.multiSelect)}
                                                    onContextMenu={showTextInputContextMenu}
                                                />
                                            </label>
                                        </div>
                                    </fieldset>
                                )}
                            </For>
                        </div>

                        <div class="agent-question-panel-actions">
                            <button
                                type="button"
                                class="agent-question-panel-btn agent-question-panel-btn--cancel"
                                onClick={defer}
                            >
                                Answer later
                            </button>
                            <button
                                type="button"
                                class="agent-question-panel-btn agent-question-panel-btn--submit"
                                disabled={!allAnswered()}
                                onClick={() => submit()}
                            >
                                Submit answer
                            </button>
                        </div>
                    </div>
                </Show>
            )}
        </Show>
    );
};

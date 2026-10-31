// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Coverage for the Tier 3 predictive-countdown addition
 * (docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md §7) —
 * explicit "~Nk to auto-compact" text, surfaced inline once the fill level
 * reaches the mid band, escalating visually at the critical band. Not a
 * full component smoke test — AgentComposerStrip's other zones (controls,
 * turn/session stats, Shell toggle) are exercised elsewhere; this file is
 * scoped to the countdown behavior this change actually adds.
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

import { AgentComposerStrip } from "./AgentComposerStrip";

// AgentRuntimeDropup (rendered via showControls()) imports this — same mock
// AgentRuntimeDropup.test.tsx uses; only exercised by the reagent P1
// regression test below, which opens that dropdown.
vi.mock("../runtime-apply", () => ({
    applyRuntimeChange: vi.fn().mockResolvedValue(undefined),
}));

afterEach(() => {
    cleanup();
});

const baseProps = {
    logOpen: false,
    onToggleLog: () => {},
    // The countdown is Claude-only (compactionThreshold() hard-codes
    // Claude Code's own auto-compact buffer) — most tests below want it
    // enabled; the provider-gating test overrides this explicitly.
    providerId: "claude",
};

// SPEC_COMPOSER_STRIP_DROP_CENTER_STATS_2026_08_31.md: the centered stats
// zone (token/elapsed readout) duplicated AgentWorkingRow's "Worked" row
// and was removed. Guards against the props/content being silently
// reintroduced later.
describe("AgentComposerStrip — no centered stats zone content (SPEC_COMPOSER_STRIP_DROP_CENTER_STATS_2026_08_31.md)", () => {
    it("never renders .agent-composer-strip-stats while loading", () => {
        const { container } = render(() => (
            <AgentComposerStrip {...baseProps} loading={true} />
        ));
        expect(container.querySelector(".agent-composer-strip-stats")).toBeNull();
    });

    it("never renders .agent-composer-strip-stats while idle", () => {
        const { container } = render(() => (
            <AgentComposerStrip {...baseProps} loading={false} />
        ));
        expect(container.querySelector(".agent-composer-strip-stats")).toBeNull();
    });
});

describe("AgentComposerStrip — the context reading and where it comes from", () => {
    it("a provider with no context reading still gets the session popover once a turn reported totals", () => {
        render(() => (
            <AgentComposerStrip
                {...baseProps}
                providerId="codex"
                contextTokens={null}
                sessionTotals={{ input_tokens: 90_000, output_tokens: 3_000 }}
            />
        ));
        expect(screen.getByRole("button", { name: "session" })).toBeTruthy();
    });

    it("no reading and no totals on another provider: no slot", () => {
        render(() => <AgentComposerStrip {...baseProps} providerId="codex" contextTokens={null} />);
        expect(screen.queryByRole("button", { name: "session" })).toBeNull();
    });

    it("shows tokens over the window, with the provenance note in the tooltip", () => {
        render(() => (
            <AgentComposerStrip
                {...baseProps}
                contextTokens={300_000}
                contextWindow={1_000_000}
                contextNote="Window reported by the CLI for claude-sonnet-5-5."
            />
        ));
        const reading = screen.getByRole("button", { name: "300k / 1.0m" });
        expect(reading.getAttribute("title")).toMatch(/Context window: 300,000 \/ 1,000,000 tokens/);
        expect(reading.getAttribute("title")).toMatch(/Window reported by the CLI for claude-sonnet-5-5\.$/);
    });

    it("shows the tokens alone, still with the note, when the window is unknown", () => {
        render(() => (
            <AgentComposerStrip {...baseProps} contextTokens={300_000} contextWindow={undefined} contextNote="Window not known for some-model." />
        ));
        const reading = screen.getByRole("button", { name: "300k ctx" });
        expect(reading.getAttribute("title")).toBe("Context: 300,000 tokens\nWindow not known for some-model.");
    });
});

describe("AgentComposerStrip — the CLI's own auto-compact point", () => {
    it("counts down to the threshold the CLI reports, and says so", () => {
        // CLAUDE_CODE_AUTO_COMPACT_WINDOW=300000: the CLI compacts at 267k, not 967k.
        render(() => (
            <AgentComposerStrip
                {...baseProps}
                contextTokens={210_000}
                contextWindow={1_000_000}
                autoCompact={{ kind: "at", tokens: 267_000, source: "reported" }}
            />
        ));
        expect(screen.getByText(/~57k to auto-compact/)).toBeInTheDocument();
        const reading = screen.getByRole("button", { name: "210k / 1.0m" });
        expect(reading.getAttribute("title")).toMatch(/Auto-compacts at 267,000 tokens \(≈57,000 tokens left\), as the CLI reports it\./);
        expect(reading.className).toMatch(/agent-composer-strip-ctx--high/);
    });

    it("shows no countdown when auto-compaction is off, and bands against the window", () => {
        render(() => (
            <AgentComposerStrip {...baseProps} contextTokens={900_000} contextWindow={1_000_000} autoCompact={{ kind: "off" }} />
        ));
        expect(screen.queryByText(/to auto-compact/)).toBeNull();
        const reading = screen.getByRole("button", { name: "900k / 1.0m" });
        expect(reading.getAttribute("title")).toMatch(/Auto-compaction is off for this session/);
        expect(reading.className).toMatch(/agent-composer-strip-ctx--critical/);
    });
});

describe("AgentComposerStrip — Tier 3 predictive countdown", () => {
    it("renders no countdown text when the window is unknown", () => {
        render(() => (
            <AgentComposerStrip {...baseProps} contextTokens={10_000} contextWindow={undefined} />
        ));
        expect(screen.queryByText(/to auto-compact/)).toBeNull();
    });

    it("renders no countdown text in the low band", () => {
        // compactionThreshold(200_000) = 167_000; 10_000 / 167_000 ≈ 6% — low band.
        render(() => (
            <AgentComposerStrip {...baseProps} contextTokens={10_000} contextWindow={200_000} />
        ));
        expect(screen.queryByText(/to auto-compact/)).toBeNull();
    });

    it("renders explicit countdown text once the mid band is reached", () => {
        // threshold = 167_000; 50% of that ≈ 83_500 — mid band.
        render(() => (
            <AgentComposerStrip {...baseProps} contextTokens={90_000} contextWindow={200_000} />
        ));
        expect(screen.getByText(/~77k to auto-compact/)).toBeInTheDocument();
    });

    it("does not apply the critical escalation class in the mid band", () => {
        const { container } = render(() => (
            <AgentComposerStrip {...baseProps} contextTokens={90_000} contextWindow={200_000} />
        ));
        expect(container.querySelector(".agent-composer-strip-ctx-countdown--critical")).toBeNull();
    });

    it("applies the critical escalation class once the critical band is crossed", () => {
        // threshold = 167_000; 90% of that ≈ 150_300 — critical band.
        const { container } = render(() => (
            <AgentComposerStrip {...baseProps} contextTokens={155_000} contextWindow={200_000} />
        ));
        expect(screen.getByText(/to auto-compact/)).toBeInTheDocument();
        expect(container.querySelector(".agent-composer-strip-ctx-countdown--critical")).not.toBeNull();
    });

    it("clamps the countdown at zero once past the threshold, never going negative", () => {
        render(() => (
            <AgentComposerStrip {...baseProps} contextTokens={200_000} contextWindow={200_000} />
        ));
        expect(screen.getByText(/~0 to auto-compact/)).toBeInTheDocument();
    });

    it("the tooltip states this predicts auto-compaction only, not a manual /compact", () => {
        const { container } = render(() => (
            <AgentComposerStrip {...baseProps} contextTokens={90_000} contextWindow={200_000} />
        ));
        const countdownEl = container.querySelector(".agent-composer-strip-ctx-countdown");
        expect(countdownEl?.getAttribute("title")).toMatch(/manual \/compact can happen at any fill level/);
    });

    // Regression for Codex P2 on PR #2729: compactionThreshold() hard-codes
    // Claude Code's own ~33K auto-compact buffer. A non-Claude provider
    // that happens to report Claude-shaped message_start usage (e.g. a
    // muxcode-catalog entry) must not get an invented countdown against a
    // threshold that was never verified for it.
    it("suppresses the countdown for a non-Claude provider even in the critical band", () => {
        render(() => (
            <AgentComposerStrip
                {...baseProps}
                providerId="codex"
                contextTokens={155_000}
                contextWindow={200_000}
            />
        ));
        expect(screen.queryByText(/to auto-compact/)).toBeNull();
    });
});

describe("AgentComposerStrip — zone-assignment identity stability (reagent P1 on PR #2808)", () => {
    it("keeps the Mode/Model/Effort dropdown open when an unrelated slot's own state changes (e.g. the ctx slot appearing)", async () => {
        // Before this fix, <For> iterated the FRESH {key,side,render}
        // objects `slots()` allocates on every recompute — any unrelated
        // prop change (contextTokens here) gave every slot a brand-new
        // object identity, so <For> destroyed and recreated ALL of them,
        // including AgentRuntimeDropup (which owns its own `open` signal).
        // That silently closed this exact dropdown. `<For>` now iterates
        // stable string keys (see `slotByKey`'s doc comment in
        // AgentComposerStrip.tsx) — this test would fail against the old
        // behavior (aria-expanded resets to "false" after setContextTokens).
        const [contextTokens, setContextTokens] = createSignal<number | null>(null);
        render(() => (
            <AgentComposerStrip
                {...baseProps}
                blockId="block-1"
                blockAtom={() => undefined}
                contextTokens={contextTokens()}
                contextWindow={200_000}
            />
        ));

        await userEvent.click(screen.getByRole("button", { name: /Runtime settings/i }));
        expect(screen.getByRole("button", { name: /Runtime settings/i }).getAttribute("aria-expanded")).toBe("true");

        setContextTokens(40_000);
        // Proves the update actually reached the DOM (the ctx slot is gated
        // on contextTokens) before trusting the assertion below —
        // otherwise a no-op re-render would make this test pass for the
        // wrong reason regardless of whether the underlying bug is fixed.
        await screen.findByText(/40k/i);

        // Deliberately RE-QUERIES rather than reusing the button reference
        // from before setContextTokens: if the fix regresses and the trigger
        // gets destroyed/recreated, the OLD (now-detached) node would still
        // report its last "aria-expanded=true" forever, making a reused
        // reference pass for the wrong reason regardless of the real bug.
        expect(screen.getByRole("button", { name: /Runtime settings/i }).getAttribute("aria-expanded")).toBe("true");
    });

    it("still updates ctx text live in place when contextTokens changes, despite the untrack() around the one-time render() call", async () => {
        // The untrack() fix above deliberately stops the OUTER slot lookup
        // from re-invoking render() on unrelated changes — this test guards
        // the other direction: it must NOT also break live reactivity
        // WITHIN a slot's own already-rendered JSX (ctx text needs to keep
        // updating every turn without the whole slot remounting).
        const [contextTokens, setContextTokens] = createSignal(90_000);
        render(() => (
            <AgentComposerStrip {...baseProps} contextTokens={contextTokens()} contextWindow={200_000} />
        ));

        expect(screen.getByText(/90k/i)).toBeInTheDocument();

        setContextTokens(120_000);
        await screen.findByText(/120k/i);

        expect(screen.queryByText(/90k/i)).not.toBeInTheDocument();
    });
});

describe("AgentComposerStrip — interactive elements flush against the row edge", () => {
    // b must FOLLOW a in document order.
    const precedes = (a: Element, b: Element) => (a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING) !== 0;

    it("renders the Compact button BEFORE the ctx text when the ctx slot sits on the left side", () => {
        const { container } = render(() => (
            <AgentComposerStrip {...baseProps} onCompact={() => {}} contextTokens={40_000} contextWindow={200_000} />
        ));
        const compact = screen.getByRole("button", { name: "Compact" });
        const ctxText = container.querySelector(".agent-composer-strip-ctx")!;
        expect(compact.closest(".agent-composer-strip-row-left")).not.toBeNull();
        expect(precedes(compact, ctxText)).toBe(true);
    });

    it("orders a passive slot INSIDE an interactive one on the left side (auth inward of the runtime trigger)", () => {
        // auth is the strip's one remaining passive slot: the ctx slot went
        // interactive when its reading became the session-stats trigger
        // (SPEC_AGENT_SHELL_DRAWER_INFO_PANEL_2026_09_19.md §3.1), so this
        // property is now exercised on the left, where auth is pinned (Rev 9,
        // SPEC_COMPOSER_STRIP_AUTH_COMPACT_SIDE_STABILITY_2026_09_16.md)
        // alongside the interactive runtime trigger.
        const { container } = render(() => (
            <AgentComposerStrip {...baseProps} blockId="block-1" blockAtom={() => undefined} authStatus="authenticated" />
        ));
        const auth = container.querySelector(".agent-composer-strip-auth")!;
        const trigger = screen.getByRole("button", { name: /Runtime settings/i });
        expect(auth.closest(".agent-composer-strip-row-left")).not.toBeNull();
        expect(precedes(trigger, auth)).toBe(true);
    });

    it("still offers the session trigger when there is history but no context reading yet", () => {
        // ReAgent P1 on #3436: Archive/Export live behind this trigger. A
        // pane that has history but has not reported context usage — the
        // interrupted / resume-failed states — must not lose access to them.
        const { container } = render(() => (
            <AgentComposerStrip
                {...baseProps}
                blockId="block-1"
                blockAtom={() => ({ meta: { "session:line_count": 4200 } }) as never}
                contextTokens={null}
            />
        ));
        const trigger = container.querySelector(".agent-session-stats-trigger")!;
        expect(trigger).not.toBeNull();
        expect(trigger).toHaveTextContent("session");
    });

    it("renders no session trigger for a pane with neither a reading nor history", () => {
        const { container } = render(() => (
            <AgentComposerStrip {...baseProps} blockId="block-1" blockAtom={() => ({ meta: {} }) as never} contextTokens={null} />
        ));
        expect(container.querySelector(".agent-session-stats-trigger")).toBeNull();
    });

    it("keeps Shell the outermost element of the right edge, outside an interactive ctx slot", () => {
        render(() => (
            <AgentComposerStrip
                {...baseProps}
                authStatus="authenticated"
                agentMode="host"
                onCompact={() => {}}
                contextTokens={40_000}
                contextWindow={200_000}
            />
        ));
        const shell = screen.getByRole("button", { name: /Shell/i });
        const compact = screen.getByRole("button", { name: "Compact" });
        expect(precedes(compact, shell)).toBe(true);
    });

    it("mirrors the hostShell slot's internal order when it lands on the LEFT (degenerate one-slot case): Shell outermost-left, NATIVE badge inward", () => {
        // Minimal pool: no controls (no blockAtom), no auth, no badge, no
        // ctx — hostShell alone falls back to the left side (the "left
        // must never be completely empty" fallback), putting it on the
        // line's LEFT edge, where Shell must flip to the outer (first)
        // position.
        const { container } = render(() => (
            <AgentComposerStrip logOpen={false} onToggleLog={() => {}} agentMode="host" />
        ));
        const shell = screen.getByRole("button", { name: /Shell/i });
        const hostBadge = container.querySelector(".agent-composer-strip .runtime-badge")!;
        expect(shell.closest(".agent-composer-strip-row-left")).not.toBeNull();
        expect(precedes(shell, hostBadge)).toBe(true);
    });
});

/**
 * The sign-in chip names the account: its email, shortened to 22 characters
 * (identity-model's shortenEmail), full address in the tooltip. The green dot
 * stays; without a recorded email it keeps saying "Logged in".
 */
describe("AgentComposerStrip — sign-in chip shows the account email", () => {
    const chip = (container: HTMLElement) => container.querySelector(".agent-composer-strip-auth")!;

    it("shows the shortened email, with the full address in the tooltip, when signed in", () => {
        const { container } = render(() => (
            <AgentComposerStrip {...baseProps} authStatus="authenticated" authEmail="jonathan.ross@anthropic.com" />
        ));
        expect(chip(container)).toHaveTextContent("jonat…ss@anthropic.com");
        expect(chip(container)).not.toHaveTextContent("Logged in");
        expect(chip(container).getAttribute("title")).toBe("Signed in as jonathan.ross@anthropic.com");
        expect(chip(container).querySelector(".agent-composer-strip-auth-dot")).not.toBeNull();
        expect(chip(container).classList.contains("agent-composer-strip-auth--ok")).toBe(true);
    });

    it("an email that fits is shown whole", () => {
        const { container } = render(() => (
            <AgentComposerStrip {...baseProps} authStatus="authenticated" authEmail="alice@example.com" />
        ));
        expect(chip(container)).toHaveTextContent("alice@example.com");
    });

    it("keeps 'Logged in' when no email is known", () => {
        const { container } = render(() => <AgentComposerStrip {...baseProps} authStatus="authenticated" />);
        expect(chip(container)).toHaveTextContent("Logged in");
        expect(chip(container).getAttribute("title")).toBe("Signed in to this agent's provider");
    });

    it("never shows an email while signed out", () => {
        const { container } = render(() => (
            <AgentComposerStrip {...baseProps} authStatus="unauthenticated" authEmail="alice@example.com" />
        ));
        expect(chip(container)).toHaveTextContent("Not logged in");
        expect(chip(container)).not.toHaveTextContent("@");
    });
});

// The chip is a "switch account" link when signed in and another account of the
// same provider exists. A switch restarts the agent, so srv holds one asked for
// during a turn or a compaction until it ends, and the chip says where it is going.
// SPEC_COMPOSER_ACCOUNT_SWITCH_AND_JEKT_HEIGHT_CAP_2026_09_26.md Part A.
describe("AgentComposerStrip — sign-in chip switches account", () => {
    const chip = (container: HTMLElement) => container.querySelector(".agent-composer-strip-auth")!;
    const link = (container: HTMLElement) =>
        container.querySelector<HTMLButtonElement>("button.agent-composer-strip-auth-link");

    it("renders the email as a button that hands the click to onSwitchAccount", async () => {
        const onSwitchAccount = vi.fn();
        const { container } = render(() => (
            <AgentComposerStrip
                {...baseProps}
                authStatus="authenticated"
                authEmail="alice@example.com"
                canSwitchAccount={true}
                onSwitchAccount={onSwitchAccount}
            />
        ));
        const btn = link(container)!;
        expect(btn).not.toBeNull();
        expect(btn).toHaveTextContent("alice@example.com");
        expect(btn.getAttribute("aria-haspopup")).toBe("menu");
        expect(chip(container).getAttribute("title")).toBe("Signed in as alice@example.com. Click to switch account.");
        expect(chip(container).querySelector(".agent-composer-strip-auth-dot")).not.toBeNull();
        expect(chip(container).classList.contains("agent-composer-strip-auth--ok")).toBe(true);

        await userEvent.click(btn);
        expect(onSwitchAccount).toHaveBeenCalledTimes(1);
        expect(onSwitchAccount.mock.calls[0][0]).toBeInstanceOf(MouseEvent);
    });

    it("an account with no recorded email is a link too", () => {
        const { container } = render(() => (
            <AgentComposerStrip
                {...baseProps}
                authStatus="authenticated"
                canSwitchAccount={true}
                onSwitchAccount={() => {}}
            />
        ));
        expect(link(container)).toHaveTextContent("Logged in");
    });

    it("a container agent's chip shows its account and switches it although the sign-in is checked later", async () => {
        const onSwitchAccount = vi.fn();
        const props = { ...baseProps, authStatus: "unknown" as const, authEmail: "alice@example.com", canSwitchAccount: true, onSwitchAccount };
        const { container } = render(() => <AgentComposerStrip {...props} signInCheckedLater={true} />);
        expect(link(container)).toHaveTextContent("alice@example.com");
        // Nothing has confirmed the login, so it is neither green nor "Signed in as".
        expect(chip(container).className).not.toContain("--ok");
        expect(chip(container).getAttribute("title")).toBe("Account alice@example.com. Click to switch account.");
        await userEvent.click(link(container)!);
        expect(onSwitchAccount).toHaveBeenCalledTimes(1);
        // A host agent's unchecked sign-in still shows no chip, so the strip doesn't flash a colour.
        const host = render(() => <AgentComposerStrip {...props} />);
        expect(link(host.container)).toBeNull();
    });

    it("a container agent with no account linked yet still gets the chip, to pick one", async () => {
        const onSwitchAccount = vi.fn();
        const props = { ...baseProps, authStatus: "unknown" as const, canSwitchAccount: true, onSwitchAccount };
        const { container } = render(() => <AgentComposerStrip {...props} signInCheckedLater={true} />);
        expect(link(container)).toHaveTextContent("Account");
        expect(chip(container).className).not.toContain("--ok");
        expect(chip(container).getAttribute("title")).toBe("No account linked yet. Click to switch account.");
        await userEvent.click(link(container)!);
        expect(onSwitchAccount).toHaveBeenCalledTimes(1);
    });

    it("stays plain text when there is no other account to switch to", () => {
        const { container } = render(() => (
            <AgentComposerStrip
                {...baseProps}
                authStatus="authenticated"
                authEmail="alice@example.com"
                canSwitchAccount={false}
                onSwitchAccount={() => {}}
            />
        ));
        expect(link(container)).toBeNull();
        expect(chip(container)).toHaveTextContent("alice@example.com");
        expect(chip(container).getAttribute("title")).toBe("Signed in as alice@example.com");
    });

    it("stays plain text when it could switch but has no handler", () => {
        const { container } = render(() => (
            <AgentComposerStrip
                {...baseProps}
                authStatus="authenticated"
                authEmail="alice@example.com"
                canSwitchAccount={true}
            />
        ));
        expect(link(container)).toBeNull();
    });

    it("is plain text, with the reason in the tooltip, while a turn is running", () => {
        const { container } = render(() => (
            <AgentComposerStrip
                {...baseProps}
                loading={true}
                authStatus="authenticated"
                authEmail="alice@example.com"
                canSwitchAccount={true}
                onSwitchAccount={() => {}}
            />
        ));
        // srv holds a switch asked for in a turn until the turn ends (agent.account.switch).
        expect(link(container)).not.toBeNull();
        expect(chip(container)).toHaveTextContent("alice@example.com");
        expect(chip(container).getAttribute("title")).toBe(
            "Signed in as alice@example.com. Click to switch account; it takes effect when the current turn ends."
        );
    });

    it("says where a queued switch is going", () => {
        const { container } = render(() => (
            <AgentComposerStrip
                {...baseProps}
                loading={true}
                authStatus="authenticated"
                authEmail="alice@example.com"
                pendingAccountLabel="bob@example.com"
                canSwitchAccount={true}
                onSwitchAccount={() => {}}
            />
        ));
        expect(chip(container)).toHaveTextContent("alice@example.com \u2192 bob@example.com");
        expect(chip(container).getAttribute("title")).toBe(
            "Signed in as alice@example.com. Switching to bob@example.com when this turn ends."
        );
    });

    it("is still a link while a compaction is in progress", () => {
        const { container } = render(() => (
            <AgentComposerStrip
                {...baseProps}
                compacting={{ startedAt: 1 } as never}
                authStatus="authenticated"
                authEmail="alice@example.com"
                canSwitchAccount={true}
                onSwitchAccount={() => {}}
            />
        ));
        expect(link(container)).not.toBeNull();
    });

    it("stays a link when the turn ends", () => {
        const [loading, setLoading] = createSignal(true);
        const { container } = render(() => (
            <AgentComposerStrip
                {...baseProps}
                loading={loading()}
                authStatus="authenticated"
                authEmail="alice@example.com"
                canSwitchAccount={true}
                onSwitchAccount={() => {}}
            />
        ));
        expect(link(container)).not.toBeNull();
        setLoading(false);
        expect(link(container)).not.toBeNull();
    });

    it("signed out with a menu: the chip is a link that opens it", () => {
        const onSwitchAccount = vi.fn();
        const { container } = render(() => (
            <AgentComposerStrip
                {...baseProps}
                authStatus="unauthenticated"
                canSwitchAccount={true}
                onSwitchAccount={onSwitchAccount}
            />
        ));
        expect(link(container)).not.toBeNull();
        expect(chip(container)).toHaveTextContent("Not logged in");
        expect(chip(container).getAttribute("title")).toBe("Not signed in — click to log in or switch account");
        link(container)!.click();
        expect(onSwitchAccount).toHaveBeenCalledTimes(1);
    });

    it("signed out with no menu is plain text", () => {
        const { container } = render(() => <AgentComposerStrip {...baseProps} authStatus="unauthenticated" />);
        expect(link(container)).toBeNull();
        expect(chip(container)).toHaveTextContent("Not logged in");
        expect(chip(container).getAttribute("title")).toBe("Not signed in — click Log in to continue");
    });
});

// A Codex pane: the Mode · Model · Effort panel, and Compact once its thread
// exists. Codex reports no context reading, so Compact sits in the slot its
// session totals open, and srv compacts the thread on "/compact".
describe("AgentComposerStrip — a Codex pane gets the runtime panel and Compact", () => {
    const codex = (meta: Record<string, unknown>, extra: Record<string, unknown> = {}) =>
        render(() => (
            <AgentComposerStrip
                {...baseProps}
                providerId="codex"
                blockId="block-1"
                blockAtom={() => ({ meta }) as never}
                onCompact={() => {}}
                sessionTotals={{ input_tokens: 12_000, output_tokens: 300 }}
                {...extra}
            />
        ));

    it("shows the runtime panel and a Compact that works once the thread has an id", () => {
        codex({ "agent:sessionid": "01a1643d-1929-7ce3-a8da-8538c0ba661e" });
        expect(screen.getByRole("button", { name: /Runtime settings/i })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Compact" })).toBeEnabled();
    });

    it("keeps Compact off before the first turn, and while a turn runs", () => {
        codex({});
        expect(screen.getByRole("button", { name: "Compact" })).toBeDisabled();
        cleanup();
        codex({ "agent:sessionid": "t1" }, { loading: true });
        expect(screen.getByRole("button", { name: "Compact" })).toBeDisabled();
    });

    it("offers no Compact in a container, where srv can't compact a Codex thread", () => {
        codex({ "agent:sessionid": "t1" }, { agentMode: "container" });
        expect(screen.queryByRole("button", { name: "Compact" })).toBeNull();
        expect(screen.getByRole("button", { name: /Runtime settings/i })).toBeInTheDocument();
    });

    it("gives neither to a provider without them", () => {
        render(() => (
            <AgentComposerStrip {...baseProps} providerId="gemini" blockId="block-1" blockAtom={() => ({ meta: {} }) as never} onCompact={() => {}} sessionTotals={{ input_tokens: 1, output_tokens: 1 }} />
        ));
        expect(screen.queryByRole("button", { name: /Runtime settings/i })).toBeNull();
        expect(screen.queryByRole("button", { name: "Compact" })).toBeNull();
    });
});

describe("AgentComposerStrip — the View chip (SPEC_AGENT_PANE_VERBOSITY_MODES_2026_10_29.md §4.5)", () => {
    it("shows no chip while the pane shows everything", () => {
        const { container } = render(() => <AgentComposerStrip {...baseProps} onOpenView={() => {}} viewChip={null} />);
        expect(container.querySelector("[data-agent-view-chip]")).toBeNull();
    });

    it("shows the view's name and opens the popover on a click", async () => {
        const onOpenView = vi.fn();
        const { container } = render(() => <AgentComposerStrip {...baseProps} onOpenView={onOpenView} viewChip="Replies · this pane" viewOpen={false} />);
        const chip = container.querySelector<HTMLElement>("[data-agent-view-chip]")!;
        expect(chip.textContent).toBe("Replies · this pane");
        expect(chip.getAttribute("aria-expanded")).toBe("false");
        await userEvent.click(chip);
        expect(onOpenView).toHaveBeenCalledTimes(1);
    });
});

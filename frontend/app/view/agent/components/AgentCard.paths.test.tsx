// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The agent picker's definition tile (AgentCard in AgentCard.tsx), path by path, beside the
// behaviours AgentCard.test.tsx already pins: what the title and caption fall back to, the install
// ribbon and the update chip, the classes and ARIA state a card carries, the narrow card's two chips,
// the default focus, a click, a key, type-to-filter (the caret, the text it inserts and where), "+ New"
// and the context menu.

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

const { providerLogoSpy, claimFocusOnMount } = vi.hoisted(() => ({
    providerLogoSpy: vi.fn(),
    claimFocusOnMount: vi.fn((_blockId: string, give: () => boolean): void => {
        give();
    }),
}));

vi.mock("@/element/ProviderLogo", () => ({
    ProviderLogo: (props: { provider: string; size?: number; class?: string }) => {
        providerLogoSpy(props);
        return null;
    },
}));
vi.mock("@/app/store/focusManager", () => ({
    focusManager: { claimFocusOnMount },
}));

import type { AgentDefinition } from "@/app/store/rpc-api";
import { AgentCard } from "./AgentCard";

afterEach(() => {
    cleanup();
    vi.clearAllMocks();
    vi.unstubAllGlobals();
});

const makeAgent = (over: Partial<AgentDefinition> = {}): AgentDefinition =>
    ({
        id: "def-1",
        slug: "mine",
        name: "My agent",
        icon: "",
        provider: "claude",
        description: "",
        working_directory: "",
        shell: "",
        provider_flags: "",
        auto_start: 0,
        restart_on_crash: 0,
        idle_timeout_minutes: 0,
        created_at: 0,
        agent_type: "",
        environment: "",
        agent_bus_id: "",
        is_seeded: 1,
        ...over,
    }) as AgentDefinition;

type Props = Partial<Parameters<typeof AgentCard>[0]>;

function mount(over: Props = {}) {
    const onLaunch = vi.fn();
    // One plain object, so a prop given as undefined stays undefined (merged props would skip it).
    const props = { agent: makeAgent(), launching: false, disabled: false, installed: true, onLaunch, ...over };
    const result = render(() => <AgentCard {...props} />);
    const card = result.container.querySelector(".agent-card") as HTMLElement;
    return { ...result, card, onLaunch: props.onLaunch as ReturnType<typeof vi.fn> };
}

const chips = (root: HTMLElement) =>
    [...root.querySelectorAll(".ui-tag--chip")].map((c) => {
        const own = c.cloneNode(true) as HTMLElement;
        own.querySelectorAll(".ui-tag-sr").forEach((n) => n.remove());
        return own.textContent?.replace(/\s+/g, " ").trim();
    });

describe("AgentCard: title and caption", () => {
    it("names a catalogued provider by its CLI, whatever the definition is called", () => {
        const { container } = mount({ agent: makeAgent({ name: "My agent", description: "Mine" }) });
        expect(container.querySelector(".agent-card-title")).toHaveTextContent(/^Claude Code$/);
        expect(container.querySelector(".agent-card-caption")).toHaveTextContent(/^Anthropic's coding agent$/);
        expect(screen.getByRole("button", { name: "Launch Claude Code" })).toBeTruthy();
    });

    it("falls back to the definition's name and description for a provider with no catalog entry", () => {
        const { container } = mount({
            agent: makeAgent({ provider: "homegrown", name: "Home", description: "Does it all" }),
        });
        expect(container.querySelector(".agent-card-title")).toHaveTextContent(/^Home$/);
        expect(container.querySelector(".agent-card-caption")).toHaveTextContent(/^Does it all$/);
    });

    it("captions it with its name when it has no description either", () => {
        const { container } = mount({ agent: makeAgent({ provider: "homegrown", name: "Home", description: "" }) });
        expect(container.querySelector(".agent-card-caption")).toHaveTextContent(/^Home$/);
    });

    it("draws the harness icon at 28px with the card's icon class", () => {
        mount({ agent: makeAgent({ provider: "codex", name: "Other" }) });
        const props = providerLogoSpy.mock.calls[0][0];
        expect(props.provider).toBe("codex");
        expect(props.size).toBe(28);
        expect(props.class).toBe("agent-card-icon");
    });
});

describe("AgentCard: classes and ARIA state", () => {
    it("is a plain, enabled, closed card by default", () => {
        const { card } = mount({ agent: makeAgent({ provider: "muse" }) });
        expect(card.className).toBe("agent-card");
        expect(card.getAttribute("role")).toBe("button");
        expect(card.getAttribute("tabindex")).toBe("0");
        expect(card.getAttribute("aria-disabled")).toBe("false");
        expect(card.hasAttribute("aria-expanded")).toBe(false);
    });

    it("marks a launching card, and shows the spinner only then", () => {
        const { card, container } = mount({ launching: true });
        expect(card).toHaveClass("agent-card--launching");
        expect(container.querySelector(".agent-card-spinner")).not.toBeNull();
        cleanup();
        const idle = mount();
        expect(idle.card).not.toHaveClass("agent-card--launching");
        expect(idle.container.querySelector(".agent-card-spinner")).toBeNull();
    });

    it("marks a disabled card, takes it out of the tab order and says so", () => {
        const { card } = mount({ disabled: true });
        expect(card).toHaveClass("agent-card--disabled");
        expect(card.getAttribute("tabindex")).toBe("-1");
        expect(card.getAttribute("aria-disabled")).toBe("true");
    });

    it("marks an open card and reports it as expanded, and a closed one as not", () => {
        const open = mount({ expanded: true });
        expect(open.card).toHaveClass("agent-card--open");
        expect(open.card.getAttribute("aria-expanded")).toBe("true");
        cleanup();
        const closed = mount({ expanded: false });
        expect(closed.card).not.toHaveClass("agent-card--open");
        expect(closed.card.getAttribute("aria-expanded")).toBe("false");
    });

    it("marks only a CLI known to be missing as needing install", () => {
        expect(mount({ installed: false }).card).toHaveClass("agent-card--needs-install");
        cleanup();
        expect(mount({ installed: true }).card).not.toHaveClass("agent-card--needs-install");
        cleanup();
        expect(mount({ installed: undefined }).card).not.toHaveClass("agent-card--needs-install");
    });

    it("marks a card that has chips, even a single one", () => {
        expect(mount({ agent: makeAgent({ provider: "claude" }) }).card).toHaveClass("agent-card--has-tags");
        cleanup();
        expect(mount({ agent: makeAgent({ provider: "opencode" }) }).card).toHaveClass("agent-card--has-tags");
    });

    it("does not count the install ribbon as a chip", () => {
        const { card, container } = mount({ agent: makeAgent({ provider: "muse" }), installed: false });
        expect(container.querySelector(".ui-tag--ribbon")).not.toBeNull();
        expect(card).not.toHaveClass("agent-card--has-tags");
        expect(card).toHaveClass("agent-card--has-action");
    });

    it('has an action for a missing CLI even while launching, but a session\'s "+ New" only when idle', () => {
        expect(mount({ installed: false, launching: true }).card).toHaveClass("agent-card--has-action");
        cleanup();
        expect(mount({ hasCurrentSession: true }).card).toHaveClass("agent-card--has-action");
        cleanup();
        expect(mount({ hasCurrentSession: true, launching: true }).card).not.toHaveClass("agent-card--has-action");
        cleanup();
        expect(mount({ hasCurrentSession: false }).card).not.toHaveClass("agent-card--has-action");
        cleanup();
        expect(mount({ installed: undefined }).card).not.toHaveClass("agent-card--has-action");
    });
});

describe("AgentCard: the ribbon and chips", () => {
    it("shows no ribbon until the CLI is known to be missing", () => {
        for (const installed of [true, undefined]) {
            cleanup();
            const { container } = mount({ installed });
            expect(container.querySelector(".ui-tag--ribbon")).toBeNull();
        }
    });

    it("shows the ribbon in the ok tone with a download icon", () => {
        const { container } = mount({ installed: false });
        const ribbon = container.querySelector(".ui-tag--ribbon") as HTMLElement;
        expect(ribbon).toHaveClass("ui-tag--ok");
        expect(ribbon.querySelector("i")).toHaveClass("fa-download");
        expect(ribbon).toHaveTextContent(/^Click to install$/);
    });

    it("says Installing… on the ribbon only while the install runs", () => {
        const running = mount({ installed: false, installing: true });
        expect(running.container.querySelector(".ui-tag--ribbon")).toHaveTextContent(/^Installing…$/);
        cleanup();
        const idle = mount({ installed: false, installing: false });
        expect(idle.container.querySelector(".ui-tag--ribbon")).toHaveTextContent(/^Click to install$/);
    });

    it("reads Free and Local from the provider, not from the definition's name", () => {
        const { container } = mount({ agent: makeAgent({ provider: "opencode", name: "Mine" }) });
        expect(chips(container)).toEqual(["Free", "Local"]);
        cleanup();
        const other = mount({ agent: makeAgent({ provider: "muse", name: "opencode" }) });
        expect(chips(other.container)).toEqual([]);
    });

    it("gives the access chips a tooltip detail and the update chip a native title", () => {
        const { container } = mount({ agent: makeAgent({ provider: "opencode" }), drift: "behind-pin" });
        expect(container.querySelector('[data-testid="agent-access-tag-detail-free"]')).not.toBeNull();
        expect(container.querySelector('[data-testid="agent-access-tag-detail-local"]')).not.toBeNull();
        const update = [...container.querySelectorAll(".ui-tag--chip")].find((c) =>
            c.textContent?.includes("Update available")
        )!;
        expect(update.getAttribute("title")).toBe(
            "A newer CLI version is pinned by AgentMux — update it in the Toolchain pane"
        );
    });

    it("shows the update chip in the warn tone with a circle-up icon", () => {
        const { container } = mount({ drift: "behind-pin" });
        const update = [...container.querySelectorAll(".ui-tag--chip")].find((c) =>
            c.textContent?.includes("Update available")
        )!;
        expect(update).toHaveClass("ui-tag--warn");
        expect(update.querySelector("i")).toHaveClass("fa-circle-up");
    });

    it("shows the update chip while the CLI hasn't been checked yet, but not once it is known to be missing", () => {
        const { container } = mount({ installed: undefined, drift: "behind-pin" });
        expect(chips(container)).toContain("Update available");
        cleanup();
        const missing = mount({ installed: false, drift: "behind-pin" });
        expect(chips(missing.container)).not.toContain("Update available");
    });

    it("gives Free a gift icon and Local a house icon", () => {
        const { container } = mount({ agent: makeAgent({ provider: "opencode" }) });
        const [free, local] = [...container.querySelectorAll(".ui-tag--chip")];
        expect(free.querySelector("i")).toHaveClass("fa-gift");
        expect(local.querySelector("i")).toHaveClass("fa-house");
    });

    it("draws the chips small", () => {
        const { container } = mount({ agent: makeAgent({ provider: "opencode" }) });
        const all = [...container.querySelectorAll(".ui-tag--chip")];
        expect(all.length).toBeGreaterThan(0);
        for (const c of all) expect(c).toHaveClass("ui-tag--xs");
        expect(container.querySelector(".agent-card-tags")).not.toBeNull();
    });
});

describe("AgentCard: a narrow card", () => {
    const watchSize = () => {
        const seen = {
            fire: undefined as undefined | ((entries: unknown[]) => void),
            observed: [] as Element[],
            disconnected: 0,
        };
        vi.stubGlobal(
            "ResizeObserver",
            class {
                constructor(cb: (entries: unknown[]) => void) {
                    seen.fire = cb;
                }
                observe(el: Element) {
                    seen.observed.push(el);
                }
                disconnect() {
                    seen.disconnected++;
                }
            }
        );
        return seen;
    };
    const width = (w: number) => [{ contentRect: { width: w } }];

    it("watches the card, and stops when it goes", () => {
        const seen = watchSize();
        const { card, unmount } = mount();
        expect(seen.observed).toEqual([card]);
        expect(seen.disconnected).toBe(0);
        unmount();
        expect(seen.disconnected).toBe(1);
    });

    it("is narrow below 300px and not from 300px up", () => {
        const seen = watchSize();
        const { card } = mount();
        seen.fire!(width(299));
        expect(card).toHaveClass("agent-card--narrow");
        seen.fire!(width(300));
        expect(card).not.toHaveClass("agent-card--narrow");
        seen.fire!(width(350));
        expect(card).not.toHaveClass("agent-card--narrow");
        seen.fire!(width(250));
        expect(card).toHaveClass("agent-card--narrow");
    });

    it("ignores a measurement with no width, and an empty one", () => {
        const seen = watchSize();
        const { card } = mount();
        seen.fire!(width(250));
        seen.fire!(width(0));
        expect(card).toHaveClass("agent-card--narrow");
        seen.fire!([]);
        expect(card).toHaveClass("agent-card--narrow");
        cleanup();
        const fresh = mount();
        seen.fire!(width(0));
        expect(fresh.card).not.toHaveClass("agent-card--narrow");
    });

    it("shows two chips and folds the rest into +N, and all three when wide", () => {
        const seen = watchSize();
        const { container } = mount({ agent: makeAgent({ provider: "opencode" }), drift: "behind-pin" });
        expect(chips(container)).toEqual(["Free", "Local", "Update available"]);
        seen.fire!(width(250));
        expect(chips(container)).toEqual(["Free", "Local", "+1"]);
        seen.fire!(width(400));
        expect(chips(container)).toEqual(["Free", "Local", "Update available"]);
    });
});

describe("AgentCard: default focus", () => {
    it("takes the focus, through the pane's guard, only when it is the default and can be launched", () => {
        const card = mount({ defaultFocus: true, blockId: "blk-1" }).card;
        expect(claimFocusOnMount).toHaveBeenCalledTimes(1);
        expect(document.activeElement).toBe(card);
        expect(claimFocusOnMount.mock.calls[0][1]()).toBe(true);
    });

    it("doesn't claim focus when it isn't the default, or has no pane", () => {
        mount({ defaultFocus: false, blockId: "blk-1" });
        cleanup();
        mount({ blockId: "blk-1" });
        cleanup();
        mount({ defaultFocus: true });
        expect(claimFocusOnMount).not.toHaveBeenCalled();
    });
});

describe("AgentCard: clicking and keys", () => {
    it("launches its agent with the click", () => {
        const { card, onLaunch } = mount();
        fireEvent.click(card);
        expect(onLaunch).toHaveBeenCalledTimes(1);
        expect(onLaunch.mock.calls[0][0]).toMatchObject({ id: "def-1" });
        expect(onLaunch.mock.calls[0][1]).toBeInstanceOf(MouseEvent);
    });

    it("does nothing on a click while disabled", () => {
        const { card, onLaunch } = mount({ disabled: true });
        fireEvent.click(card);
        expect(onLaunch).not.toHaveBeenCalled();
    });

    it("launches on Enter and on Space, with the key event, and keeps the page from scrolling on Space", () => {
        const { card, onLaunch } = mount();
        expect(fireEvent.keyDown(card, { key: "Enter" })).toBe(false);
        expect(fireEvent.keyDown(card, { key: " " })).toBe(false);
        expect(onLaunch).toHaveBeenCalledTimes(2);
        expect(onLaunch.mock.calls[0][0]).toMatchObject({ id: "def-1" });
        expect(onLaunch.mock.calls[0][1]).toBeInstanceOf(KeyboardEvent);
        expect(onLaunch.mock.calls[1][1]).toBeInstanceOf(KeyboardEvent);
    });

    it("leaves other keys alone", () => {
        const { card, onLaunch } = mount();
        expect(fireEvent.keyDown(card, { key: "ArrowDown" })).toBe(true);
        expect(fireEvent.keyDown(card, { key: "Tab" })).toBe(true);
        expect(onLaunch).not.toHaveBeenCalled();
    });

    it("does nothing on Enter or Space while disabled, and doesn't swallow them", () => {
        const { card, onLaunch } = mount({ disabled: true });
        expect(fireEvent.keyDown(card, { key: "Enter" })).toBe(true);
        expect(fireEvent.keyDown(card, { key: " " })).toBe(true);
        expect(onLaunch).not.toHaveBeenCalled();
    });

    it("doesn't launch on a key pressed in a chip", () => {
        const { container, onLaunch } = mount({ agent: makeAgent({ provider: "opencode" }) });
        const chip = container.querySelector(".agent-card-tags .ui-tag") as HTMLElement;
        fireEvent.keyDown(chip, { key: "Enter" });
        fireEvent.click(chip);
        expect(onLaunch).not.toHaveBeenCalled();
    });

    it("launches on a click in the card's text, though not in a chip", () => {
        const { container, onLaunch } = mount({ agent: makeAgent({ provider: "opencode" }) });
        fireEvent.click(container.querySelector(".agent-card-title")!);
        expect(onLaunch).toHaveBeenCalledTimes(1);
    });
});

describe("AgentCard: type to filter", () => {
    const inPane = (over: Props = {}, filterValue = "") => {
        const onLaunch = vi.fn();
        const onInput = vi.fn();
        const result = render(() => (
            <div data-blockid="blk-other">
                <input class="other-filter" data-pane-focus />
                <div data-blockid="blk-1" onInput={onInput}>
                    <input class="plain" />
                    <input class="filter" data-pane-focus value={filterValue} />
                    <AgentCard
                        agent={makeAgent()}
                        launching={false}
                        disabled={false}
                        installed={true}
                        onLaunch={onLaunch}
                        {...over}
                    />
                </div>
            </div>
        ));
        const card = result.container.querySelector(".agent-card") as HTMLElement;
        const filter = result.container.querySelector<HTMLInputElement>("input.filter")!;
        const other = result.container.querySelector<HTMLInputElement>("input.other-filter")!;
        return { ...result, card, filter, other, onLaunch, onInput };
    };

    it("types a printable key at the end of what the filter already has, with the caret after it", () => {
        const { card, filter } = inPane({}, "ab");
        card.focus();
        expect(fireEvent.keyDown(card, { key: "c" })).toBe(false);
        expect(filter.value).toBe("abc");
        expect(filter.selectionStart).toBe(3);
        expect(filter.selectionEnd).toBe(3);
        expect(document.activeElement).toBe(filter);
    });

    it("lets the pane see the input, as it would a typed one", () => {
        const { card, onInput } = inPane();
        fireEvent.keyDown(card, { key: "c" });
        expect(onInput).toHaveBeenCalledTimes(1);
    });

    it("uses its own pane's filter, and the one marked for focus", () => {
        const { card, filter, other } = inPane();
        fireEvent.keyDown(card, { key: "c" });
        expect(filter.value).toBe("c");
        expect(other.value).toBe("");
        expect(document.querySelector<HTMLInputElement>("input.plain")!.value).toBe("");
    });

    it("moves the focus without scrolling", () => {
        const { card, filter } = inPane();
        const focus = vi.spyOn(filter, "focus");
        fireEvent.keyDown(card, { key: "c" });
        expect(focus).toHaveBeenCalledWith({ preventScroll: true });
    });

    it("leaves Space, a shortcut and a named key to the card", () => {
        const { card, filter } = inPane();
        card.focus();
        for (const init of [
            { key: " " },
            { key: "a", ctrlKey: true },
            { key: "a", metaKey: true },
            { key: "a", altKey: true },
            { key: "Escape" },
            { key: "" },
        ]) {
            expect(fireEvent.keyDown(card, init)).toBe(init.key === " " ? false : true);
        }
        expect(filter.value).toBe("");
        expect(document.activeElement).toBe(card);
    });

    it("types nothing, and swallows nothing, when the card has no filter", () => {
        const onLaunch = vi.fn();
        const { container } = render(() => (
            <AgentCard agent={makeAgent()} launching={false} disabled={false} installed={true} onLaunch={onLaunch} />
        ));
        const card = container.querySelector(".agent-card") as HTMLElement;
        expect(fireEvent.keyDown(card, { key: "c" })).toBe(true);
        expect(onLaunch).not.toHaveBeenCalled();
    });

    it("sends a composition to the filter by any of its three signs, and inserts nothing", () => {
        for (const init of [{ key: "a", isComposing: true }, { key: "Process" }, { key: "a", keyCode: 229 }]) {
            const { card, filter, unmount } = inPane();
            card.focus();
            fireEvent.keyDown(card, init);
            expect(document.activeElement).toBe(filter);
            expect(filter.value).toBe("");
            unmount();
        }
    });

    it("moves a composition's caret to the filter without scrolling", () => {
        const { card, filter } = inPane();
        const focus = vi.spyOn(filter, "focus");
        fireEvent.keyDown(card, { key: "Process", keyCode: 229 });
        expect(focus).toHaveBeenCalledWith({ preventScroll: true });
    });

    it("doesn't launch on the Enter that ends a composition when there is a filter to take it", () => {
        const { card, onLaunch } = inPane();
        fireEvent.keyDown(card, { key: "Enter", isComposing: true });
        expect(onLaunch).not.toHaveBeenCalled();
    });

    it("launches on that Enter when there is no filter", () => {
        const onLaunch = vi.fn();
        const { container } = render(() => (
            <AgentCard agent={makeAgent()} launching={false} disabled={false} installed={true} onLaunch={onLaunch} />
        ));
        fireEvent.keyDown(container.querySelector(".agent-card")!, { key: "Enter", isComposing: true });
        expect(onLaunch).toHaveBeenCalledTimes(1);
    });
});

describe("AgentCard: + New and the context menu", () => {
    const onNewSession = vi.fn();

    it('shows "+ New" for a card with a session, as a compact button named for its agent', () => {
        mount({ hasCurrentSession: true, onNewSession });
        const button = screen.getByRole("button", {
            name: "Start a new session for Claude Code (archives the current one)",
        });
        expect(button).toHaveTextContent(/^\+ New$/);
        expect(button).toHaveClass("agent-card-new-session-btn", "ui-density-compact");
        expect(button.getAttribute("title")).toBe("Archive current session and start a new one");
        expect(button.getAttribute("tabindex")).toBe("0");
    });

    it("shows it while the CLI hasn't been checked, but not when it's missing, launching, or the card has no session", () => {
        const text = () => screen.queryByText("+ New");
        mount({ hasCurrentSession: true, installed: undefined });
        expect(text()).not.toBeNull();
        cleanup();
        mount({ hasCurrentSession: true, installed: false });
        expect(text()).toBeNull();
        cleanup();
        mount({ hasCurrentSession: true, launching: true });
        expect(text()).toBeNull();
        cleanup();
        mount({ hasCurrentSession: false });
        expect(text()).toBeNull();
        cleanup();
        mount();
        expect(text()).toBeNull();
    });

    it("asks for a new session without launching the card", () => {
        onNewSession.mockClear();
        const { onLaunch } = mount({ hasCurrentSession: true, onNewSession });
        fireEvent.click(screen.getByText("+ New"));
        expect(onNewSession).toHaveBeenCalledTimes(1);
        expect(onNewSession.mock.calls[0][0]).toMatchObject({ id: "def-1" });
        expect(onLaunch).not.toHaveBeenCalled();
    });

    it("does nothing, and still doesn't launch, while disabled", () => {
        onNewSession.mockClear();
        const { onLaunch } = mount({ hasCurrentSession: true, disabled: true, onNewSession });
        const button = screen.getByText("+ New");
        expect(button.getAttribute("tabindex")).toBe("-1");
        fireEvent.click(button);
        expect(onNewSession).not.toHaveBeenCalled();
        expect(onLaunch).not.toHaveBeenCalled();
    });

    it("clicks without a handler as a no-op", () => {
        const { onLaunch } = mount({ hasCurrentSession: true });
        fireEvent.click(screen.getByText("+ New"));
        expect(onLaunch).not.toHaveBeenCalled();
    });

    it("hands the context menu to its handler with the agent, and leaves the browser's menu to it", () => {
        const onContextMenu = vi.fn();
        const { card } = mount({ onContextMenu });
        expect(fireEvent.contextMenu(card)).toBe(true);
        expect(onContextMenu).toHaveBeenCalledTimes(1);
        expect(onContextMenu.mock.calls[0][0]).toMatchObject({ id: "def-1" });
        expect(onContextMenu.mock.calls[0][1]).toBeInstanceOf(MouseEvent);
    });

    it("leaves the browser's own menu alone when no handler is wired", () => {
        const { card } = mount();
        expect(fireEvent.contextMenu(card)).toBe(true);
    });
});

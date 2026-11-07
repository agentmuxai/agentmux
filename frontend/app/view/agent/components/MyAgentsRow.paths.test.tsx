// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** One tile of the My agents list (MyAgentsRow), path by path: the classes the tile takes (open, active, running
 *  with no pane), the logo, the entry button and the chevron that open the panel, the runtime chip, the no-pane
 *  tag and the active badge with where it is open, the runtime and seat badges, the account, the seat line, the
 *  preview or what stands in for it, the Vault asks, the message count, and the three times and what shows each.
 *  The chips, badges, logo, tooltip and time formatting are stubs that print what they were given, so what is
 *  checked is only what this component decides. Written ahead of splitting `MyAgentsRow`. */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { RecentSessionRow } from "@/types/rpc/RecentSessionRow";

vi.mock("@/app/element/tooltip", () => ({
    Tooltip: (p: { content: string; divClassName?: string; children: unknown }) => (
        <div data-testid="tooltip" data-content={p.content} class={p.divClassName}>
            {p.children as never}
        </div>
    ),
}));
vi.mock("@/app/store/agent-runtime", () => ({
    hasProcess: (state: string) => state === "running" || state === "idle",
}));
vi.mock("@/element/DualProviderLogo", () => ({
    DualProviderLogo: (p: { harness: string; vendor: string; size: number; class?: string }) => (
        <div data-testid="logo" data-harness={p.harness} data-vendor={p.vendor} data-size={p.size} class={p.class} />
    ),
}));
vi.mock("../providers/catalog", () => ({
    resolveEffectiveVendor: (provider: string, url: string) => `vendor(${provider}|${url})`,
}));
vi.mock("../../../agent-model/seat-format", () => ({
    seatBadgeText: (s: { role: string }) => `badge:${s.role}`,
    seatReportsLine: (s: { role: string }) => `reports:${s.role}`,
    seatTooltipLine: (s: { role: string }) => `tip:${s.role}`,
}));
vi.mock("./AgentRuntimeChip", () => ({
    AgentRuntimeChip: (p: { state: string; detail?: string; blocked?: string }) => (
        <div
            data-testid="runtime-chip"
            data-state={p.state}
            data-detail={p.detail ?? ""}
            data-blocked={p.blocked ?? ""}
        />
    ),
}));
vi.mock("./RuntimeBadge", () => ({
    RuntimeBadge: (p: { runtime: string; size?: string }) => (
        <div data-testid="runtime-badge" data-runtime={p.runtime} data-size={p.size ?? ""} />
    ),
}));
vi.mock("./my-agents-list-text", () => ({
    noPreviewText: (row: { definition_id: string }) => `nopreview:${row.definition_id}`,
}));
vi.mock("./vault-request-card/vault-row-asks", () => ({
    VaultRowAsks: (p: { agent?: string }) => <div data-testid="vault-asks" data-agent={p.agent ?? "none"} />,
}));
vi.mock("@/util/format-time", () => ({ formatTimeAgo: (ts: number, now: number) => `ago(${ts},${now})` }));

import type { MyAgentsListProps } from "./MyAgentsList";
import { MyAgentsRow } from "./MyAgentsRow";

const ID = "def-1";

const makeRow = (over: Partial<RecentSessionRow> = {}): RecentSessionRow =>
    ({
        instance_id: "inst-1",
        instance_name: "Nickname",
        definition_id: ID,
        definition_name: "Definition",
        provider: "claude",
        model_vendor_base_url: "",
        identity_name: "",
        preview: "",
        node_count: 0,
        last_active_at: 0,
        has_snapshot: false,
        agent_created_at: 0,
        started_at: 0,
        agent_type: "",
        ...over,
    }) as RecentSessionRow;

interface Setup {
    row?: Partial<RecentSessionRow>;
    open?: boolean;
    location?: string;
    runtime?: { state: string; detail?: string; blocked?: string; agent_id?: string } | undefined;
    noListFns?: boolean;
    expanded?: boolean;
    now?: number;
}

function mount(opts: Setup = {}) {
    const [expanded, setExpanded] = createSignal(opts.expanded ?? false);
    const [open, setOpen] = createSignal(opts.open ?? false);
    const [runtime, setRuntime] = createSignal(opts.runtime);
    const [now, setNow] = createSignal(opts.now ?? 1000);
    const onToggle = vi.fn();
    const tileRef = vi.fn();
    const base = {
        onReattach: vi.fn(),
        openDefinitions: () => new Map(open() ? [[ID, "block-1"]] : []),
        openLocations: () => new Map(opts.location ? [[ID, { label: opts.location }]] : []),
        runtimeOf: (id: string) => (id === ID ? runtime() : undefined),
    };
    const list = (opts.noListFns ? { onReattach: vi.fn() } : base) as unknown as MyAgentsListProps;
    const outer = vi.fn();
    const host = document.createElement("div");
    document.body.appendChild(host);
    // A parent that listens for clicks through Solid, as the list does: the chevron must not let one through.
    render(
        () => (
            <ul onClick={outer}>
                <MyAgentsRow
                    row={makeRow(opts.row)}
                    list={list}
                    now={now}
                    isExpanded={expanded}
                    onToggle={onToggle}
                    tileRef={tileRef}
                />
            </ul>
        ),
        { container: host }
    );
    const q = (sel: string) => host.querySelector(sel) as HTMLElement | null;
    const all = (sel: string) => Array.from(host.querySelectorAll(sel)) as HTMLElement[];
    const text = (sel: string) => q(sel)?.textContent ?? null;
    return { q, all, text, onToggle, tileRef, outer, setExpanded, setOpen, setRuntime, setNow, tile: () => q("li")! };
}

afterEach(() => {
    cleanup();
    document.body.innerHTML = "";
});

describe("the tile", () => {
    it("is a list item for the definition, handed to the list as its tile", () => {
        const m = mount();
        const li = m.tile();
        expect(li.tagName).toBe("LI");
        expect(li.classList.contains("agent-recent-sessions-row")).toBe(true);
        expect(li.getAttribute("data-definition-id")).toBe(ID);
        expect(m.tileRef).toHaveBeenCalledTimes(1);
        expect(m.tileRef.mock.calls[0][0]).toBe(li);
    });

    it("takes no state class when closed, not open elsewhere and not running", () => {
        const li = mount().tile();
        expect(li.className).toBe("agent-recent-sessions-row");
    });

    it("is expanded while its panel is open, and follows it", () => {
        const m = mount({ expanded: true });
        expect(m.tile().classList.contains("is-expanded")).toBe(true);
        m.setExpanded(false);
        expect(m.tile().classList.contains("is-expanded")).toBe(false);
    });

    it("is active when its definition is open in a pane, and follows it", () => {
        const m = mount({ open: true });
        expect(m.tile().classList.contains("agent-recent-sessions-row--active")).toBe(true);
        m.setOpen(false);
        expect(m.tile().classList.contains("agent-recent-sessions-row--active")).toBe(false);
    });

    it("is not active when the list says nothing about open definitions", () => {
        const m = mount({ noListFns: true });
        expect(m.tile().classList.contains("agent-recent-sessions-row--active")).toBe(false);
    });

    it("is marked no-pane when the agent has a process and no pane shows it", () => {
        const m = mount({ runtime: { state: "running" } });
        expect(m.tile().classList.contains("agent-recent-sessions-row--no-pane")).toBe(true);
    });

    it("is not no-pane when a pane shows it", () => {
        const m = mount({ runtime: { state: "running" }, open: true });
        expect(m.tile().classList.contains("agent-recent-sessions-row--no-pane")).toBe(false);
    });

    it("is not no-pane when the agent has no process, or no runtime is known", () => {
        const stopped = mount({ runtime: { state: "stopped" } });
        expect(stopped.tile().classList.contains("agent-recent-sessions-row--no-pane")).toBe(false);
        cleanup();
        const unknown = mount({ runtime: undefined });
        expect(unknown.tile().classList.contains("agent-recent-sessions-row--no-pane")).toBe(false);
        cleanup();
        const noFns = mount({ noListFns: true });
        expect(noFns.tile().classList.contains("agent-recent-sessions-row--no-pane")).toBe(false);
    });

    it("keeps the state classes together, each from its own condition", () => {
        const m = mount({ expanded: true, open: true, runtime: { state: "running" } });
        const cls = Array.from(m.tile().classList);
        expect(cls).toEqual(expect.arrayContaining(["is-expanded", "agent-recent-sessions-row--active"]));
        expect(cls).not.toContain("agent-recent-sessions-row--no-pane");
    });

    it("follows the runtime as it changes", () => {
        const m = mount({ runtime: { state: "stopped" } });
        expect(m.tile().classList.contains("agent-recent-sessions-row--no-pane")).toBe(false);
        m.setRuntime({ state: "idle" });
        expect(m.tile().classList.contains("agent-recent-sessions-row--no-pane")).toBe(true);
    });
});

describe("the logo and the entry", () => {
    it("shows the provider's logo at 40 with the vendor it resolves to", () => {
        const m = mount({ row: { provider: "codex", model_vendor_base_url: "https://v.test" } });
        const logo = m.q('[data-testid="logo"]')!;
        expect(logo.getAttribute("data-harness")).toBe("codex");
        expect(logo.getAttribute("data-vendor")).toBe("vendor(codex|https://v.test)");
        expect(logo.getAttribute("data-size")).toBe("40");
        expect(logo.classList.contains("agent-recent-sessions-icon")).toBe(true);
    });

    it("names the entry by the instance, else the definition", () => {
        const named = mount();
        const entry = named.q('[data-testid="agent-my-agents-entry"]')!;
        expect(entry.textContent).toBe("Nickname");
        expect(entry.getAttribute("aria-label")).toBe("Nickname");
        expect(entry.querySelector(".agent-recent-sessions-name")!.textContent).toBe("Nickname");
        cleanup();
        const bare = mount({ row: { instance_name: "" } });
        const e2 = bare.q('[data-testid="agent-my-agents-entry"]')!;
        expect(e2.textContent).toBe("Definition");
        expect(e2.getAttribute("aria-label")).toBe("Definition");
    });

    it("makes the entry the row's stretched button, expanded with the panel", () => {
        const m = mount();
        const entry = m.q('[data-testid="agent-my-agents-entry"]')!;
        expect(entry.tagName).toBe("BUTTON");
        expect(entry.classList.contains("agent-recent-sessions-entry")).toBe(true);
        expect(entry.classList.contains("ui-button--row")).toBe(true);
        expect(entry.classList.contains("ui-density-compact")).toBe(true);
        expect(entry.getAttribute("aria-expanded")).toBe("false");
        m.setExpanded(true);
        expect(entry.getAttribute("aria-expanded")).toBe("true");
    });

    it("opens the panel when the entry is clicked", () => {
        const m = mount();
        fireEvent.click(m.q('[data-testid="agent-my-agents-entry"]')!);
        expect(m.onToggle).toHaveBeenCalledTimes(1);
    });

    it("keeps the entry and the badges together on the first line", () => {
        const m = mount({
            open: true,
            runtime: { state: "idle" },
            row: { agent_type: "host", seat: { role: "worker" } as never },
        });
        const line1 = m.q(".agent-recent-sessions-line1")!;
        const kids = Array.from(line1.children).map((c) => c.getAttribute("data-testid") ?? c.className);
        expect(kids[0]).toBe("agent-my-agents-entry");
        expect(line1.querySelector('[data-testid="runtime-chip"]')).not.toBeNull();
        expect(line1.querySelector(".agent-active-badge")).not.toBeNull();
        expect(line1.querySelector('[data-testid="runtime-badge"]')).not.toBeNull();
        expect(line1.querySelector(".agent-seat-badge")).not.toBeNull();
    });
});

describe("the chip, the tag and the active badge", () => {
    it("shows the runtime chip with the state, detail and what blocks it", () => {
        const m = mount({ runtime: { state: "running", detail: "starting", blocked: "no key" } });
        const chip = m.q('[data-testid="runtime-chip"]')!;
        expect(chip.getAttribute("data-state")).toBe("running");
        expect(chip.getAttribute("data-detail")).toBe("starting");
        expect(chip.getAttribute("data-blocked")).toBe("no key");
    });

    it("shows no chip without a runtime, or without the list's lookup", () => {
        expect(mount({ runtime: undefined }).q('[data-testid="runtime-chip"]')).toBeNull();
        cleanup();
        expect(mount({ noListFns: true }).q('[data-testid="runtime-chip"]')).toBeNull();
    });

    it("tags a running agent that has no pane", () => {
        const m = mount({ runtime: { state: "running" } });
        const tag = m.q(".agent-no-pane-tag")!;
        expect(tag.textContent).toBe("no pane");
    });

    it("does not tag an agent a pane shows, or one that is not running", () => {
        expect(mount({ runtime: { state: "running" }, open: true }).q(".agent-no-pane-tag")).toBeNull();
        cleanup();
        expect(mount({ runtime: { state: "stopped" } }).q(".agent-no-pane-tag")).toBeNull();
    });

    it("badges an active agent with where it is open, capitalised", () => {
        const m = mount({ open: true, location: "this window" });
        const tip = m.q('[data-testid="tooltip"]')!;
        expect(tip.getAttribute("data-content")).toBe("This window");
        expect(tip.classList.contains("agent-active-badge-anchor")).toBe(true);
        const badge = tip.querySelector(".agent-active-badge")!;
        expect(badge.getAttribute("aria-label")).toBe("Active");
    });

    it("says 'Open in another pane' when it does not know where", () => {
        const m = mount({ open: true });
        expect(m.q('[data-testid="tooltip"]')!.getAttribute("data-content")).toBe("Open in another pane");
    });

    it("shows no active badge when the list gives no open definitions", () => {
        const m = mount({ noListFns: true });
        expect(m.q(".agent-active-badge")).toBeNull();
    });

    it("shows no badge when the agent is not open in a pane", () => {
        const m = mount({ location: "this window" });
        expect(m.q(".agent-active-badge")).toBeNull();
        expect(m.q('[data-testid="tooltip"]')).toBeNull();
    });
});

describe("the runtime and seat badges", () => {
    it("shows the runtime badge for a host agent, as a tag", () => {
        const b = mount({ row: { agent_type: "host" } }).q('[data-testid="runtime-badge"]')!;
        expect(b.getAttribute("data-runtime")).toBe("host");
        expect(b.getAttribute("data-size")).toBe("tag");
    });

    it("shows it for a container agent", () => {
        const b = mount({ row: { agent_type: "container" } }).q('[data-testid="runtime-badge"]')!;
        expect(b.getAttribute("data-runtime")).toBe("container");
    });

    it("shows none for a kind it does not know", () => {
        expect(mount({ row: { agent_type: "standalone" } }).q('[data-testid="runtime-badge"]')).toBeNull();
        cleanup();
        expect(mount({ row: { agent_type: "" } }).q('[data-testid="runtime-badge"]')).toBeNull();
    });

    it("shows the seat as a badge with its tooltip line", () => {
        const m = mount({ row: { seat: { role: "worker" } as never } });
        const pill = m.q(".agent-seat-badge")!;
        expect(pill.textContent).toBe("badge:worker");
        expect(pill.getAttribute("title")).toBe("tip:worker");
    });

    it("shows no seat badge without a seat", () => {
        expect(mount().q(".agent-seat-badge")).toBeNull();
    });
});

describe("the account, the seat line and the preview", () => {
    it("shows the account on its own line, below the first", () => {
        const m = mount({ row: { identity_name: "Work" } });
        const acct = m.q(".agent-recent-sessions-account")!;
        expect(acct.textContent).toBe("Work");
        expect(acct.parentElement!.classList.contains("agent-recent-sessions-body")).toBe(true);
        expect(acct.previousElementSibling!.classList.contains("agent-recent-sessions-line1")).toBe(true);
    });

    it("shows no account line when there is none", () => {
        expect(mount().q(".agent-recent-sessions-account")).toBeNull();
    });

    it("shows the seat's reporting line", () => {
        const m = mount({ row: { seat: { role: "lead" } as never } });
        const line = m.q('[data-testid="agent-my-agents-seat-line"]')!;
        expect(line.textContent).toBe("reports:lead");
        expect(line.classList.contains("agent-recent-sessions-seat")).toBe(true);
    });

    it("shows no seat line without a seat", () => {
        expect(mount().q('[data-testid="agent-my-agents-seat-line"]')).toBeNull();
    });

    it("shows the preview when there is one", () => {
        const m = mount({ row: { preview: "fix the build" } });
        const p = m.q(".agent-recent-sessions-preview")!;
        expect(p.textContent).toBe("fix the build");
        expect(p.classList.contains("agent-recent-sessions-preview--empty")).toBe(false);
    });

    it("says why there is none when the preview is empty", () => {
        const m = mount();
        const p = m.q(".agent-recent-sessions-preview")!;
        expect(p.textContent).toBe(`nopreview:${ID}`);
        expect(p.classList.contains("agent-recent-sessions-preview--empty")).toBe(true);
        expect(m.all(".agent-recent-sessions-preview")).toHaveLength(1);
    });

    it("lays the lines out in order: first line, account, seat, preview, asks, count", () => {
        const m = mount({ row: { identity_name: "Work", seat: { role: "w" } as never, preview: "p", node_count: 2 } });
        const body = m.q(".agent-recent-sessions-body")!;
        const order = Array.from(body.children).map((c) => c.getAttribute("data-testid") ?? c.className);
        expect(order).toEqual([
            "agent-recent-sessions-line1",
            "agent-recent-sessions-account",
            "agent-my-agents-seat-line",
            "agent-recent-sessions-preview",
            "vault-asks",
            "agent-recent-sessions-line3",
        ]);
    });
});

describe("the Vault asks", () => {
    it("asks for the agent's asks when it runs without a pane", () => {
        const m = mount({ runtime: { state: "running", agent_id: "ag-1" } });
        expect(m.q('[data-testid="vault-asks"]')!.getAttribute("data-agent")).toBe("ag-1");
    });

    it("passes no agent when a pane shows it", () => {
        const m = mount({ runtime: { state: "running", agent_id: "ag-1" }, open: true });
        expect(m.q('[data-testid="vault-asks"]')!.getAttribute("data-agent")).toBe("none");
    });

    it("passes no agent when it is not running", () => {
        const m = mount({ runtime: { state: "stopped", agent_id: "ag-1" } });
        expect(m.q('[data-testid="vault-asks"]')!.getAttribute("data-agent")).toBe("none");
    });

    it("passes no agent when no runtime is known", () => {
        const m = mount({ noListFns: true });
        expect(m.q('[data-testid="vault-asks"]')!.getAttribute("data-agent")).toBe("none");
    });
});

describe("the message count", () => {
    it("shows none when the snapshot had no nodes", () => {
        expect(mount().q(".agent-recent-sessions-line3")).toBeNull();
    });

    it("says '1 message'", () => {
        const m = mount({ row: { node_count: 1 } });
        expect(m.text(".agent-recent-sessions-nodes")!.replace(/\s+/g, " ").trim()).toBe("1 message");
    });

    it("says 'N messages' for any other count", () => {
        const m = mount({ row: { node_count: 12 } });
        expect(m.text(".agent-recent-sessions-nodes")!.replace(/\s+/g, " ").trim()).toBe("12 messages");
    });
});

describe("the times", () => {
    const stamps = (m: ReturnType<typeof mount>) =>
        m
            .all(".agent-recent-sessions-ts")
            .map((el) => [
                el.querySelector(".agent-recent-sessions-ts-label")!.textContent,
                el.querySelector(".agent-recent-sessions-ts-value")!.textContent,
            ]);

    it("shows none for a row with no times", () => {
        const m = mount();
        expect(m.q(".agent-recent-sessions-timestamps")).not.toBeNull();
        expect(stamps(m)).toEqual([]);
    });

    it("shows Last Active, Last Launch and Created, in that order, each aged against now", () => {
        const m = mount({
            now: 9000,
            row: { has_snapshot: true, started_at: 100, last_active_at: 500, agent_created_at: 50 },
        });
        expect(stamps(m)).toEqual([
            ["Last Active", "ago(500,9000)"],
            ["Last Launch", "ago(100,9000)"],
            ["Created", "ago(50,9000)"],
        ]);
    });

    it("follows the clock", () => {
        const m = mount({ now: 1, row: { started_at: 100 } });
        expect(stamps(m)).toEqual([["Last Launch", "ago(100,1)"]]);
        m.setNow(2);
        expect(stamps(m)).toEqual([["Last Launch", "ago(100,2)"]]);
    });

    it("hides Last Active without a snapshot", () => {
        const m = mount({ row: { has_snapshot: false, started_at: 100, last_active_at: 500 } });
        expect(stamps(m).map((s) => s[0])).toEqual(["Last Launch"]);
    });

    it("hides Last Active when it is not after the launch", () => {
        const same = mount({ row: { has_snapshot: true, started_at: 100, last_active_at: 100 } });
        expect(stamps(same).map((s) => s[0])).toEqual(["Last Launch"]);
        cleanup();
        const before = mount({ row: { has_snapshot: true, started_at: 100, last_active_at: 90 } });
        expect(stamps(before).map((s) => s[0])).toEqual(["Last Launch"]);
    });

    it("hides Last Active, and Last Launch, when the agent never launched", () => {
        const m = mount({ row: { has_snapshot: true, started_at: 0, last_active_at: 500, agent_created_at: 50 } });
        expect(stamps(m).map((s) => s[0])).toEqual(["Created"]);
    });

    it("shows a time of one millisecond, as the first moment", () => {
        const m = mount({ row: { started_at: 1, agent_created_at: 1 } });
        expect(stamps(m).map((s) => s[0])).toEqual(["Last Launch", "Created"]);
    });

    it("hides Created when it has no creation time", () => {
        const m = mount({ row: { started_at: 100 } });
        expect(stamps(m).map((s) => s[0])).toEqual(["Last Launch"]);
    });
});

describe("the chevron", () => {
    it("is its own button beside the entry, named for the agent", () => {
        const m = mount();
        const t = m.q('[data-testid="agent-my-agents-menu-toggle"]')!;
        expect(t.tagName).toBe("BUTTON");
        expect(t.getAttribute("aria-label")).toBe("Actions for Nickname");
        expect(t.classList.contains("agent-recent-sessions-menu-toggle")).toBe(true);
        expect(t.closest('[data-testid="agent-my-agents-entry"]')).toBeNull();
        expect(t.parentElement).toBe(m.tile());
        expect(t.classList.contains("ui-density-compact")).toBe(true);
        expect(t.querySelector(".fa-chevron-down")).not.toBeNull();
        expect(t.querySelector(".fa-chevron-up")).toBeNull();
    });

    it("is named for the definition when there is no instance name", () => {
        const m = mount({ row: { instance_name: "" } });
        expect(m.q('[data-testid="agent-my-agents-menu-toggle"]')!.getAttribute("aria-label")).toBe(
            "Actions for Definition"
        );
    });

    it("shows open and expanded with the panel", () => {
        const m = mount();
        const t = m.q('[data-testid="agent-my-agents-menu-toggle"]')!;
        expect(t.classList.contains("is-open")).toBe(false);
        expect(t.getAttribute("aria-expanded")).toBe("false");
        m.setExpanded(true);
        expect(t.classList.contains("is-open")).toBe(true);
        expect(t.getAttribute("aria-expanded")).toBe("true");
    });

    it("toggles the panel without the click reaching the tile's parents", () => {
        const m = mount();
        fireEvent.click(m.q('[data-testid="agent-my-agents-menu-toggle"]')!);
        expect(m.onToggle).toHaveBeenCalledTimes(1);
        expect(m.outer).not.toHaveBeenCalled();
    });

    it("lets a click on the entry reach the parents", () => {
        const m = mount();
        fireEvent.click(m.q('[data-testid="agent-my-agents-entry"]')!);
        expect(m.outer).toHaveBeenCalledTimes(1);
    });

    it("comes last in the tile, after the times", () => {
        const m = mount();
        const kids = Array.from(m.tile().children).map((c) => c.getAttribute("data-testid") ?? c.className);
        expect(kids).toEqual([
            "logo",
            "agent-recent-sessions-body",
            "agent-recent-sessions-timestamps",
            "agent-my-agents-menu-toggle",
        ]);
    });
});

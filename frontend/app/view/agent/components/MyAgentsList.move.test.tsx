// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A resize that changes the list's column count moves a row's panel to the last tile of that row, which
 * builds the panel again under another tile. The Seat and Environment views must keep what was typed and not
 * saved through the move (task #15600), and must still start from what is stored on a new visit.
 */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { RecentSessionRow } from "@/types/rpc/RecentSessionRow";

const listRecent = vi.fn();
const listAgents = vi.fn();
const getContent = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ListRecentSessionsCommand: (...args: unknown[]) => listRecent(...args),
        ListAgentDefinitionsCommand: (...args: unknown[]) => listAgents(...args),
        GetAgentContentCommand: (...args: unknown[]) => getContent(...args),
        AgentSetSeatCommand: vi.fn(),
        SetAgentContentCommand: vi.fn(),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => () => {} }));
vi.mock("@/element/ProviderLogo", () => ({ ProviderLogo: () => null }));
vi.mock("./AgentBundlesTab", () => ({ AgentBundlesTab: () => <div /> }));
vi.mock("@floating-ui/dom", () => ({
    autoUpdate: vi.fn(() => vi.fn()),
    computePosition: vi.fn(() => Promise.resolve({ x: 0, y: 0 })),
    flip: vi.fn(() => ({})),
    offset: vi.fn(() => ({})),
    shift: vi.fn(() => ({})),
}));

import { MyAgentsList } from "./MyAgentsList";

const row = (id: string, name: string): RecentSessionRow => ({
    instance_id: `inst-${id}`,
    instance_name: name,
    definition_id: id,
    definition_name: name,
    provider: "claude",
    working_directory: "/tmp/x",
    identity_id: "id-work",
    identity_name: "Work",
    memory_id: "",
    memory_name: "",
    block_id_hint: "",
    preview: "",
    node_count: 0,
    last_active_at: 0,
    has_snapshot: false,
    agent_created_at: 0,
    started_at: 0,
    model_vendor_base_url: "",
    session_id: "",
    snapshot_check_failed: false,
    agent_type: "host",
});

/** The list measures its column count from the grid's computed template; here the test sets it. */
let columns = 1;
const observers = new Set<() => void>();
const realComputedStyle = window.getComputedStyle.bind(window);

const resizeTo = (n: number): void => {
    columns = n;
    for (const cb of [...observers]) cb();
};

/** The tile the open panel is rendered after: its previous sibling in the list. */
const tileBeforePanel = (): string => {
    const panel = screen.getByTestId("agent-row-panel");
    return panel.previousElementSibling?.querySelector(".agent-recent-sessions-name")?.textContent ?? "";
};

describe("MyAgentsList — a panel that moves to another row", () => {
    beforeEach(() => {
        listRecent.mockReset();
        listAgents.mockReset();
        getContent.mockReset();
        columns = 1;
        listRecent.mockResolvedValue({ rows: [row("uid-a", "Alpha"), row("uid-b", "Beta")], degraded: [] });
        listAgents.mockResolvedValue([
            { id: "uid-a", name: "Alpha", is_seeded: 0 },
            { id: "uid-b", name: "Beta", is_seeded: 0 },
        ]);
        getContent.mockResolvedValue({ agent_id: "uid-a", content_type: "env", content: "A=1\n", updated_at: 1 });
        vi.stubGlobal(
            "ResizeObserver",
            class {
                cb: () => void;
                constructor(cb: () => void) {
                    this.cb = cb;
                }
                observe() {
                    observers.add(this.cb);
                }
                disconnect() {
                    observers.delete(this.cb);
                }
            }
        );
        vi.spyOn(window, "getComputedStyle").mockImplementation((el: Element, pseudo?: string | null) => {
            const real = realComputedStyle(el, pseudo);
            if (!el.classList.contains("agent-recent-sessions-list")) return real;
            return new Proxy(real, {
                get: (target, prop) =>
                    prop === "gridTemplateColumns"
                        ? Array.from({ length: columns }, () => "200px").join(" ")
                        : Reflect.get(target, prop),
            });
        });
    });
    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
        vi.restoreAllMocks();
    });

    const openView = async (testId: string) => {
        render(() => <MyAgentsList onReattach={() => {}} />);
        const user = userEvent.setup();
        const toggles = await screen.findAllByTestId("agent-my-agents-menu-toggle");
        await user.click(toggles[0]);
        await user.click(await screen.findByTestId(testId));
        return user;
    };

    it("keeps unsaved Seat edits across the move", async () => {
        await openView("agent-my-agents-seat");
        await waitFor(() => expect(screen.getByLabelText("Area")).toBeInTheDocument());
        fireEvent.input(screen.getByLabelText("Area"), { target: { value: "panes and tabs" } });
        fireEvent.input(screen.getByLabelText("Qualifier"), { target: { value: "product" } });
        expect(tileBeforePanel()).toBe("Alpha");

        resizeTo(2);

        // The panel is now after Beta (the last tile of the row), built again, and still shows the edits.
        await waitFor(() => expect(tileBeforePanel()).toBe("Beta"));
        expect(screen.getByTestId("agent-row-panel").dataset.view).toBe("seat");
        expect((screen.getByLabelText("Area") as HTMLInputElement).value).toBe("panes and tabs");
        expect((screen.getByLabelText("Qualifier") as HTMLInputElement).value).toBe("product");
        // And again when it moves back.
        resizeTo(1);
        await waitFor(() => expect(tileBeforePanel()).toBe("Alpha"));
        expect((screen.getByLabelText("Area") as HTMLInputElement).value).toBe("panes and tabs");
    });

    it("keeps unsaved Environment edits across the move", async () => {
        const user = await openView("agent-my-agents-env");
        const box = () => screen.getByLabelText("Environment") as HTMLTextAreaElement;
        await waitFor(() => expect(box().value).toBe("A=1\n"));
        fireEvent.input(box(), { target: { value: "A=1\nB=2\n" } });
        expect(tileBeforePanel()).toBe("Alpha");

        resizeTo(2);

        await waitFor(() => expect(tileBeforePanel()).toBe("Beta"));
        await waitFor(() => expect(getContent).toHaveBeenCalledTimes(2));
        expect(box().value).toBe("A=1\nB=2\n");
        // Still unsaved: Save is on.
        expect((screen.getByRole("button", { name: "Save" }) as HTMLButtonElement).disabled).toBe(false);
        await user.click(screen.getByTestId("agent-row-panel-back"));
    });

    it("starts from what is stored on a new visit, not from an earlier unsaved edit", async () => {
        const user = await openView("agent-my-agents-env");
        const box = () => screen.getByLabelText("Environment") as HTMLTextAreaElement;
        await waitFor(() => expect(box().value).toBe("A=1\n"));
        fireEvent.input(box(), { target: { value: "A=1\nB=2\n" } });

        // Back to the actions drops the edit, as it always did.
        await user.click(screen.getByTestId("agent-row-panel-back"));
        await user.click(await screen.findByTestId("agent-my-agents-env"));
        await waitFor(() => expect(box().value).toBe("A=1\n"));
    });
});

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { opened, webClick, notify } = vi.hoisted(() => ({
    opened: vi.fn(async (): Promise<{ ok: true } | { ok: false; reason: string }> => ({ ok: true })),
    notify: vi.fn(),
    webClick: vi.fn(),
}));

type Entry = { id: string; slug: string; name: string };

// A reactive roster, as the real store is, so a change reaches the index.
vi.mock("@/app/store/agent-names", async () => {
    const { createSignal } = await import("solid-js");
    const [agentRoster, setRoster] = createSignal<Entry[]>([]);
    return { followAgentNames: () => {}, agentRoster, setRoster };
});
vi.mock("@/app/store/srv-info", () => ({ srvInfo: () => ({ hostName: "narko" }) }));
vi.mock("@/app/store/global", () => ({ pushNotification: notify }));
vi.mock("@/app/store/float-modifier", () => ({ takeFloatRequest: () => false }));
vi.mock("@/app/element/link-open", () => ({
    onLinkClick: (e: MouseEvent, href: string) => {
        e.preventDefault();
        webClick(href);
    },
    onLinkAuxClick: () => {},
}));
vi.mock("./entities/registry", async (orig) => ({ ...(await orig<typeof import("./entities/registry")>()), openEntity: opened }));

import * as names from "@/app/store/agent-names";
import { AgentLink, Linkified } from "./linkified";
import { publishTasks } from "./task-index-store";

const surface = { repo: "agentmuxai/agentmux" };
const setRoster = (names as unknown as { setRoster: (e: Entry[]) => void }).setRoster;

beforeEach(() => {
    setRoster([{ id: "opaz-0909m", slug: "opaz", name: "Opaz" }]);
    publishTasks([{ board: "main", number: 48247, title: "t", state: "open" }], "main");
    opened.mockClear();
    webClick.mockClear();
    notify.mockClear();
});
afterEach(cleanup);

describe("Linkified", () => {
    it("makes an agent, a task and a web address three links in one sentence, and keeps the words", () => {
        const { container } = render(() => <Linkified text="I asked Opaz to send #48247 (https://example.com/a) today." surface={surface} />);
        const links = [...container.querySelectorAll("a")];
        expect(links.map((a) => a.textContent)).toEqual(["Opaz", "#48247", "https://example.com/a"]);
        expect(links.map((a) => a.getAttribute("aria-label"))).toEqual(["Open agent Opaz", "Open task 48247", "Open https://example.com/a"]);
        expect(container.textContent).toBe("I asked Opaz to send #48247 (https://example.com/a) today.");
    });

    it("opens a task through its kind, and keeps the press from reaching the card around it", () => {
        const around = vi.fn();
        const { container } = render(() => (
            <div onClick={around} onKeyDown={around}>
                <Linkified text="see #48247" surface={surface} />
            </div>
        ));
        const a = container.querySelector("a") as HTMLAnchorElement;
        fireEvent.click(a);
        fireEvent.keyDown(a, { key: "Enter" });
        fireEvent.keyDown(a, { key: " " });
        expect(opened).toHaveBeenCalledWith({ kind: "task", number: 48247, board: undefined }, { floating: false });
        expect(around).not.toHaveBeenCalled();
    });

    it("tells the person why a link could not open, instead of doing nothing", async () => {
        opened.mockResolvedValueOnce({ ok: false, reason: "That agent runs on another computer" });
        const { container } = render(() => <Linkified text="see #48247" surface={surface} />);
        fireEvent.click(container.querySelector("a") as HTMLAnchorElement);
        await vi.waitFor(() => expect(notify).toHaveBeenCalledTimes(1));
        expect(notify.mock.calls[0][0]).toMatchObject({ title: "Couldn't open #48247", message: "That agent runs on another computer", type: "error" });
    });

    it("opens an agent through its kind", () => {
        const { container } = render(() => <Linkified text="Opaz" surface={surface} />);
        fireEvent.click(container.querySelector("a") as HTMLAnchorElement);
        expect(opened).toHaveBeenCalledWith({ kind: "agent", id: "opaz-0909m", name: "Opaz" }, { floating: false });
    });

    it("sends a web address through the way every web link opens", () => {
        const { container } = render(() => <Linkified text="https://example.com/a" surface={surface} />);
        fireEvent.click(container.querySelector("a") as HTMLAnchorElement);
        expect(webClick).toHaveBeenCalledWith("https://example.com/a");
        expect(opened).not.toHaveBeenCalled();
    });

    it("leaves a text with nothing to link as it was, and a name no longer in the roster as text", () => {
        setRoster([]);
        const { container } = render(() => <Linkified text="Opaz thinks #999999 is fine" surface={{}} />);
        expect(container.querySelectorAll("a")).toHaveLength(0);
        expect(container.textContent).toBe("Opaz thinks #999999 is fine");
    });

    it("links a known agent as the agent, with no recognition", () => {
        const { container } = render(() => <AgentLink id="u-1" name="Tess" />);
        const a = container.querySelector("a") as HTMLAnchorElement;
        expect(a.textContent).toBe("Tess");
        fireEvent.click(a);
        expect(opened).toHaveBeenCalledWith({ kind: "agent", id: "u-1", name: "Tess" }, { floating: false });
    });
});

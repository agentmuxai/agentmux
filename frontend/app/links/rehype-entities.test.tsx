// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { openAgent } = vi.hoisted(() => ({ openAgent: vi.fn(async () => ({ ok: true as const })) }));

type Entry = { id: string; slug: string; name: string };

vi.mock("@/app/store/agent-names", async () => {
    const { createSignal } = await import("solid-js");
    const [agentRoster, setRoster] = createSignal<Entry[]>([]);
    return { followAgentNames: () => {}, agentRoster, setRoster };
});
vi.mock("@/app/store/srv-info", () => ({ srvInfo: () => ({ hostName: "narko" }) }));
vi.mock("./entities/openers", async (orig) => ({ ...(await orig<typeof import("./entities/openers")>()), openAgentEntity: openAgent }));

import * as names from "@/app/store/agent-names";
import { Markdown } from "@/app/element/markdown";
import { publishTasks } from "./task-index-store";

const setRoster = (names as unknown as { setRoster: (e: Entry[]) => void }).setRoster;
const surface = { author: { kind: "agent" as const, id: "pane-1", confined: false } };

const mount = (text: string, extra: { streaming?: boolean; entities?: typeof surface | undefined } = { entities: surface }) =>
    render(() => <Markdown text={text} scrollable={false} entities={extra.entities} streaming={extra.streaming} />).container;

beforeEach(() => {
    setRoster([{ id: "opaz-0909m", slug: "opaz", name: "Opaz" }]);
    publishTasks([{ board: "main", number: 48247, title: "t", state: "open" }], "main");
    openAgent.mockClear();
});
afterEach(cleanup);

describe("the files an agent's message names", () => {
    it("are file links, also a bare name that reads as a web domain, and never a web address", () => {
        const c = mount("See findings.md, setup.py, docs/specs/X.md and ./src/a.tsx:12 for it.");
        const files = [...c.querySelectorAll("a.entity-link--file")].map((a) => a.textContent);
        expect(files).toEqual(["findings.md", "setup.py", "docs/specs/X.md", "./src/a.tsx:12"]);
        expect([...c.querySelectorAll("a")].filter((a) => /^https?:/.test(a.getAttribute("href") ?? ""))).toEqual([]);
    });

    it("leave a real web address and a domain alone, and a surface with no agent pane keeps the old linking", () => {
        const c = mount("Docs at https://example.com/docs/a.md and example.com.");
        expect([...c.querySelectorAll("a")].map((a) => a.getAttribute("href"))).toEqual(["https://example.com/docs/a.md", "https://example.com"]);
        const plain = mount("See findings.md", { entities: undefined });
        expect(plain.querySelector("a.entity-link--file")).toBeNull();
    });

    it("are not made links for text from another computer, which keeps the web linker's reading", () => {
        const remote = { author: { kind: "agent" as const, id: "pane-1", confined: false, remote: true } };
        expect(mount("See findings.md", { entities: remote as unknown as typeof surface }).querySelector("a.entity-link--file")).toBeNull();
    });
});

describe("agent names and task numbers in a message", () => {
    it("are links with their class, address and accessible name, and the words are kept", () => {
        const c = mount("I asked Opaz about #48247 today.");
        const links = [...c.querySelectorAll("a.entity-link")];
        expect(links.map((a) => a.textContent)).toEqual(["Opaz", "#48247"]);
        expect(links.map((a) => a.getAttribute("href"))).toEqual(["agentmux://agent/opaz-0909m", "agentmux://task/48247"]);
        expect(links.map((a) => a.getAttribute("aria-label"))).toEqual(["Open agent Opaz", "Open task 48247"]);
        expect(c.textContent).toContain("I asked Opaz about #48247 today.");
    });

    it("are found inside emphasis and lists, and not inside code or an existing link", () => {
        const c = mount("- **Opaz** has #48247\n- `Opaz #48247`\n- [Opaz](https://example.com/o)\n\n```\nOpaz #48247\n```");
        expect([...c.querySelectorAll("a.entity-link")].map((a) => a.textContent)).toEqual(["Opaz", "#48247"]);
        expect(c.querySelector("code a, pre a")).toBeNull();
        expect(c.querySelector('a[href="https://example.com/o"]')?.classList.contains("entity-link")).toBe(false);
    });

    it("link nothing without a surface, and nothing that the boards and the roster do not know", () => {
        expect(mount("Opaz #48247", { entities: undefined }).querySelector("a")).toBeNull();
        expect(mount("Zed took #99").querySelector("a")).toBeNull();
    });

    it("do not wait for the end of a settled message, and do wait for it while the message arrives", () => {
        expect(mount("ask Opaz", { entities: surface, streaming: false }).querySelector("a.entity-link")).not.toBeNull();
        expect(mount("ask Opaz", { entities: surface, streaming: true }).querySelector("a.entity-link")).toBeNull();
        expect(mount("ask Opaz now", { entities: surface, streaming: true }).querySelector("a.entity-link")).not.toBeNull();
    });

    it("open the agent's pane on a click", () => {
        const c = mount("ask Opaz now");
        fireEvent.click(c.querySelector("a.entity-link") as HTMLAnchorElement);
        return vi.waitFor(() => expect(openAgent).toHaveBeenCalledWith({ kind: "agent", id: "opaz-0909m", name: "opaz-0909m" }));
    });

    it("keep only the entity classes a message may carry", () => {
        const c = mount('<a class="entity-link rainbow" href="https://example.com/x">x</a>');
        expect([...(c.querySelector("a") as HTMLElement).classList]).toEqual(["entity-link"]);
    });
});

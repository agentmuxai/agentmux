// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * OriginContainer: everything that is not the human's own words is one container whose header comes
 * from the signed origin (SPEC_MESSAGE_ORIGIN_AND_ONE_CONTAINER_2026_10_25.md §4), shown as text, and
 * never in the human's style.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it } from "vitest";
import type { SystemNoteNode } from "../types";
import { OriginContainer, lateBy } from "./OriginContainer";

afterEach(() => cleanup());

const node = (over: Partial<SystemNoteNode> = {}): SystemNoteNode => ({
    type: "system_note",
    id: "note_1",
    text: "AgentMux was upgraded from 0.60.0 to 0.60.1 at 14:17.",
    ...over,
});
const label = (n: SystemNoteNode) => render(() => <OriginContainer node={n} />).container.querySelector(".agent-origin__label")?.textContent ?? "";

describe("OriginContainer", () => {
    it("shows an AgentMux note under an AgentMux label, in its own container", () => {
        const { container } = render(() => <OriginContainer node={node()} />);
        const root = container.querySelector(".agent-origin")!;
        expect(root.getAttribute("data-origin")).toBe("system");
        expect(root.querySelector(".agent-origin__label")?.textContent).toContain("AgentMux");
        expect(root.querySelector(".agent-origin__text")?.textContent).toBe("AgentMux was upgraded from 0.60.0 to 0.60.1 at 14:17.");
    });

    it("is never the human's turn", () => {
        for (const n of [node(), node({ source: "earlier" }), node({ origin: { kind: "agent", from: "agent1" } })]) {
            cleanup();
            const { container } = render(() => <OriginContainer node={n} />);
            expect(container.querySelector(".agent-user-message")).toBeNull();
            expect(container.querySelector(".agent-origin")?.className).not.toContain("user");
        }
    });

    it("labels each origin from what srv signed", () => {
        expect(label(node({ origin: { kind: "service", from: "github-consumer" } }))).toContain("Notice from github-consumer");
        cleanup();
        expect(label(node({ origin: { kind: "schedule", from: "nightly" } }))).toContain("Scheduled: nightly");
        cleanup();
        expect(label(node({ origin: { kind: "task" } }))).toContain("Task notice");
        cleanup();
        expect(label(node({ origin: { kind: "supervisor" }, detail: "drive 2 of 5" }))).toContain("Supervisor");
        cleanup();
        expect(label(node({ source: "cli" }))).toContain("CLI message");
        cleanup();
        expect(label(node({ source: "earlier" }))).toContain("Earlier message");
    });

    it("shows a trust tag and the signature state, warning when nothing proved the sender", () => {
        const { container } = render(() => (
            <OriginContainer node={node({ origin: { kind: "service", from: "x", trust: "network-claimed", sig: "invalid" } })} />
        ));
        const tags = [...container.querySelectorAll(".ui-tag")];
        expect(tags.map((t) => t.textContent)).toEqual(["network-claimed", "signature invalid"]);
        expect(tags.every((t) => t.classList.contains("ui-tag--warn"))).toBe(true);
        cleanup();
        const ok = render(() => <OriginContainer node={node({ origin: { kind: "service", trust: "lan-verified", sig: "verified" } })} />);
        expect(ok.container.querySelector(".ui-tag--warn")).toBeNull();
        expect([...ok.container.querySelectorAll(".ui-tag")].map((t) => t.textContent)).toEqual(["lan-verified", "signed"]);
    });

    it("says when it arrived late, from the typed times or the parsed prefix", () => {
        expect(lateBy({ kind: "agent", sent_ms: 1000, delivered_ms: 1000 + 11 * 60_000 })).toBe("11 min late");
        expect(lateBy({ kind: "agent", sent_ms: 1000, delivered_ms: 91_000 })).toBeUndefined();
        const { container } = render(() => <OriginContainer node={node({ delay: "sent 14:30, delivered 14:41" })} />);
        expect(container.querySelector(".agent-origin__delay")?.textContent).toBe("sent 14:30, delivered 14:41");
    });

    it("reads the body as text, never as markup", () => {
        const { container } = render(() => <OriginContainer node={node({ text: "<b>bold</b> <img src=x onerror=alert(1)>" })} />);
        expect(container.querySelector(".agent-origin__text b")).toBeNull();
        expect(container.querySelector("img")).toBeNull();
        expect(container.querySelector(".agent-origin__text")?.textContent).toContain("<b>bold</b>");
    });

    it("folds a long body from another origin until opened; a note stays whole", () => {
        const long = "x".repeat(400);
        const { container } = render(() => <OriginContainer node={node({ text: long, origin: { kind: "schedule" } })} />);
        expect(container.querySelector(".agent-origin--folded")).not.toBeNull();
        fireEvent.click(container.querySelector(".agent-origin__more")!);
        expect(container.querySelector(".agent-origin--folded")).toBeNull();
        cleanup();
        const note = render(() => <OriginContainer node={node({ text: long })} />);
        expect(note.container.querySelector(".agent-origin--folded")).toBeNull();
        expect(note.container.querySelector(".agent-origin__more")).toBeNull();
    });

    it("a long CLI message opens fully: the fold is the only clamp", () => {
        const { container } = render(() => <OriginContainer node={node({ text: "line\n".repeat(30), source: "cli" })} />);
        expect(container.querySelector(".agent-origin--folded")).not.toBeNull();
        fireEvent.click(container.querySelector(".agent-origin__more")!);
        const root = container.querySelector(".agent-origin")!;
        expect(root.classList.contains("agent-origin--folded")).toBe(false);
        expect(container.querySelector(".agent-origin__more")?.textContent).toBe("Show less");
    });
});

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";

import { createAddressAutocomplete, inlineCompletion } from "./address-autocomplete";

const match = (over: Partial<BrowserAutocompleteMatch> = {}): BrowserAutocompleteMatch => ({
    contents: "github.com",
    description: "GitHub",
    url: "https://github.com/",
    fill_into_edit: "github.com",
    inline_autocompletion: "hub.com",
    type: "history-url",
    provider: "HistoryURL",
    relevance: 1413,
    allowed_to_be_default: true,
    ...over,
});

/** A query whose answers the test releases one at a time. */
function controlledQuery() {
    const calls: { text: string; resolve: (r: BrowserAutocompleteResult) => void }[] = [];
    const query = vi.fn(
        (text: string) => new Promise<BrowserAutocompleteResult>((resolve) => calls.push({ text, resolve }))
    );
    return { query, calls };
}

const settle = () => new Promise((r) => setTimeout(r, 0));

describe("createAddressAutocomplete", () => {
    it("shows the answer for the text it asked about", async () => {
        const { query, calls } = controlledQuery();
        const ac = createAddressAutocomplete(query);
        ac.update("git");
        calls[0].resolve({ supported: true, matches: [match()] });
        await settle();
        expect(ac.matches()).toHaveLength(1);
        expect(ac.matchesFor()).toBe("git");
    });

    it("keeps one query in flight and asks only for the newest text next", async () => {
        const { query, calls } = controlledQuery();
        const ac = createAddressAutocomplete(query);
        ac.update("g");
        ac.update("gi");
        ac.update("git");
        expect(query).toHaveBeenCalledTimes(1);
        calls[0].resolve({ supported: true, matches: [match({ contents: "for g" })] });
        await settle();
        // The answer for "g" is stale: never shown. "gi" was skipped.
        expect(ac.matches()).toEqual([]);
        expect(calls.map((c) => c.text)).toEqual(["g", "git"]);
        calls[1].resolve({ supported: true, matches: [match()] });
        await settle();
        expect(ac.matchesFor()).toBe("git");
    });

    it("stays open on the previous list while the next answer is on its way, without acting on it", async () => {
        const { query, calls } = controlledQuery();
        const ac = createAddressAutocomplete(query);
        expect(ac.open()).toBe(false);
        ac.update("git");
        calls[0].resolve({ supported: true, matches: [match()] });
        await settle();
        expect(ac.open()).toBe(true);
        // The next key: the list stays on screen (no close-and-reopen flash
        // through the page) but Enter doesn't pick its stale default.
        ac.update("gith");
        expect(ac.open()).toBe(true);
        expect(ac.chosen("gith")).toBeUndefined();
        expect(inlineCompletion("gith", { matches: ac.matches(), matchesFor: ac.matchesFor() }, false)).toBe("");
        calls[1].resolve({ supported: true, matches: [match({ contents: "github.com/x" })] });
        await settle();
        expect(ac.matchesFor()).toBe("gith");
        expect(ac.chosen("gith")?.contents).toBe("github.com/x");
        // No suggestions for the new text closes it.
        ac.update("githzz");
        calls[2].resolve({ supported: true, matches: [] });
        await settle();
        expect(ac.open()).toBe(false);
    });

    it.each([
        ["no answer", (c: { resolve: (r: any) => void }) => c.resolve(undefined)],
        ["no autocomplete", (c: { resolve: (r: any) => void }) => c.resolve({ supported: false, matches: [] })],
    ])("closes the previous list when the newest text gets %s", async (_name, answer) => {
        const { query, calls } = controlledQuery();
        const ac = createAddressAutocomplete(query);
        ac.update("git");
        calls[0].resolve({ supported: true, matches: [match()] });
        await settle();
        ac.update("gith");
        answer(calls[1]);
        await settle();
        expect(ac.open()).toBe(false);
    });

    it("closes the previous list when the query for the newest text fails", async () => {
        const query = vi
            .fn<(text: string) => Promise<BrowserAutocompleteResult | undefined>>()
            .mockResolvedValueOnce({ supported: true, matches: [match()] })
            .mockRejectedValueOnce(new Error("host gone"));
        const ac = createAddressAutocomplete(query);
        ac.update("git");
        await settle();
        expect(ac.open()).toBe(true);
        ac.update("gith");
        await settle();
        expect(ac.open()).toBe(false);
    });

    it("stops asking once the host has no autocomplete", async () => {
        const { query, calls } = controlledQuery();
        const ac = createAddressAutocomplete(query);
        ac.update("git");
        calls[0].resolve({ supported: false, matches: [] });
        await settle();
        expect(ac.supported()).toBe(false);
        ac.update("gith");
        expect(query).toHaveBeenCalledTimes(1);
    });

    it("chooses the highlighted row, else the engine's default match, for the text the list answers", async () => {
        const { query, calls } = controlledQuery();
        const ac = createAddressAutocomplete(query);
        ac.update("git");
        const top = match();
        const suggest = match({
            contents: "gitlab",
            url: "https://www.google.com/search?q=gitlab",
            type: "search-suggest",
            allowed_to_be_default: false,
        });
        calls[0].resolve({ supported: true, matches: [top, suggest] });
        await settle();
        expect(ac.chosen("git")).toBe(top);
        ac.move(1);
        ac.move(1);
        expect(ac.chosen("git")).toBe(suggest);
        // A list for other text is never acted on.
        expect(ac.chosen("gi")).toBeUndefined();
    });

    it("has no default when the top match isn't allowed to be one", async () => {
        const { query, calls } = controlledQuery();
        const ac = createAddressAutocomplete(query);
        ac.update("weather in");
        calls[0].resolve({ supported: true, matches: [match({ allowed_to_be_default: false })] });
        await settle();
        expect(ac.chosen("weather in")).toBeUndefined();
    });

    it("moves the highlight through the rows and back to none", async () => {
        const { query, calls } = controlledQuery();
        const ac = createAddressAutocomplete(query);
        ac.update("git");
        calls[0].resolve({ supported: true, matches: [match(), match()] });
        await settle();
        ac.move(1);
        expect(ac.selected()).toBe(0);
        ac.move(1);
        expect(ac.selected()).toBe(1);
        ac.move(1);
        expect(ac.selected()).toBe(-1);
        ac.move(-1);
        expect(ac.selected()).toBe(1);
    });

    it("closes on empty text", async () => {
        const { query, calls } = controlledQuery();
        const ac = createAddressAutocomplete(query);
        ac.update("git");
        calls[0].resolve({ supported: true, matches: [match()] });
        await settle();
        ac.update("   ");
        expect(ac.matches()).toEqual([]);
    });
});

describe("inlineCompletion", () => {
    const list = (m: BrowserAutocompleteMatch, matchesFor = "git") => ({ matches: [m], matchesFor });

    it("completes the default match inline", () => {
        expect(inlineCompletion("git", list(match()), false)).toBe("hub.com");
    });

    it("never completes while deleting, for stale text, or for a non-default match", () => {
        expect(inlineCompletion("git", list(match()), true)).toBe("");
        expect(inlineCompletion("gi", list(match()), false)).toBe("");
        expect(inlineCompletion("git", list(match({ allowed_to_be_default: false })), false)).toBe("");
        expect(inlineCompletion("git", list(match({ inline_autocompletion: "" })), false)).toBe("");
    });
});

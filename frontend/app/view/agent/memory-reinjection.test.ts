// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
    buildMemoryReinjectionNode,
    buildMemoryReinjectionNodeFromReplay,
    composeReinjectionMessage,
    GLOBAL_SECTION_SEPARATOR,
    RUNNING_SUMMARY_HEADING,
    isMemoryReinjectionMessage,
    memoryReinjectionNodeId,
    memorySizeBand,
    parseReinjectionMessage,
    shouldReinject,
    type MemoryEntryInput,
} from "./memory-reinjection";

// Fixtures mirror real shapes: `sizeBytes` is caller-supplied, never derived
// from `body.length`, matching agent_native_memory.rs's own real-size-vs-
// content-length distinction (see this module's header comment).
function globalEntry(label: string, body: string, sizeBytes = body.length): MemoryEntryInput {
    return { label, source: "global", body, sizeBytes };
}
function personalEntry(label: string, body: string, sizeBytes = body.length): MemoryEntryInput {
    return { label, source: "personal", body, sizeBytes };
}

describe("memorySizeBand", () => {
    // Revised 2026-09-22, second pass: bands Personal memory's ESTIMATED
    // TOKEN total against a FRACTION OF THE PANE'S CONTEXT WINDOW, mirroring
    // ctxBand()'s own denominator choice (AgentComposerStrip.tsx:550-556),
    // not an arbitrary absolute byte number. Same 0.5/0.75/0.9 boundary
    // fractions, applied to `contextWindow * fraction` instead of a flat
    // "healthy byte count" — see SPEC_HIDDEN_MEMORY_REINJECTION_AFTER_
    // COMPACTION_2026_09_22.md §3.4.2 for the full rationale (a fixed byte
    // number reads identically on a 32K-context pane and a 200K-context
    // one; a window-relative fraction does not).
    const contextWindow = 10_000;
    const fraction = 0.1; // -> healthyThreshold = 1000 tokens, same numbers as the old byte-based test for easy comparison

    it("is low from 0 up to (not including) 50% of the threshold", () => {
        expect(memorySizeBand(0, contextWindow, fraction)).toBe("low");
        expect(memorySizeBand(499, contextWindow, fraction)).toBe("low");
    });

    it("is mid from 50% up to (not including) 75%", () => {
        expect(memorySizeBand(500, contextWindow, fraction)).toBe("mid");
        expect(memorySizeBand(749, contextWindow, fraction)).toBe("mid");
    });

    it("is high from 75% up to (not including) 90%", () => {
        expect(memorySizeBand(750, contextWindow, fraction)).toBe("high");
        expect(memorySizeBand(899, contextWindow, fraction)).toBe("high");
    });

    it("is critical from 90% and beyond, including past 100%", () => {
        expect(memorySizeBand(900, contextWindow, fraction)).toBe("critical");
        expect(memorySizeBand(1000, contextWindow, fraction)).toBe("critical");
        expect(memorySizeBand(5000, contextWindow, fraction)).toBe("critical");
    });

    it("defaults the fraction to 0.10 when not supplied — the spec's proposed default", () => {
        // Same math as the explicit-fraction cases above, but relying on
        // the default: 10% of a 10,000-token window is the same 1000-token
        // threshold, so 900 tokens should read "critical" with no third arg.
        expect(memorySizeBand(900, contextWindow)).toBe("critical");
        expect(memorySizeBand(499, contextWindow)).toBe("low");
    });

    it("scales with a larger context window — the whole point of this design", () => {
        // The SAME 900-token memory total that was "critical" against a
        // 10,000-token window must read "low" against a 200,000-token one —
        // this is the exact scaling behavior a fixed byte/token threshold
        // could not produce, and the reason this spec revised away from one.
        expect(memorySizeBand(900, 200_000, fraction)).toBe("low");
    });
});

describe("shouldReinject", () => {
    it("is false when there are no entries at all — §3.1 suppression rule", () => {
        expect(shouldReinject([])).toBe(false);
    });

    it("is true with only global entries", () => {
        expect(shouldReinject([globalEntry("a", "x")])).toBe(true);
    });

    it("is true with only personal entries", () => {
        expect(shouldReinject([personalEntry("a", "x")])).toBe(true);
    });
});

describe("composeReinjectionMessage", () => {
    it("wraps content in <system-reminder> tags — §3.1", () => {
        const msg = composeReinjectionMessage([globalEntry("g1", "global body")], "compaction");
        expect(msg.startsWith("<system-reminder>")).toBe(true);
        expect(msg.trimEnd().endsWith("</system-reminder>")).toBe(true);
    });

    it("includes every entry's full body verbatim — never truncated, per the 'has to read it all' requirement", () => {
        const longBody = "x".repeat(5000);
        const msg = composeReinjectionMessage([personalEntry("big", longBody)], "compaction");
        expect(msg).toContain(longBody);
    });

    it("separates Global and Personal sections and reports each count", () => {
        const msg = composeReinjectionMessage([
            globalEntry("g1", "gbody1"),
            globalEntry("g2", "gbody2"),
            personalEntry("p1", "pbody1"),
        ], "compaction");
        expect(msg).toMatch(/Global Memory \(2 entr(y|ies)\)/);
        expect(msg).toMatch(/Personal Memory \(1 entr(y|ies)\)/);
    });

    it("omits a source's section entirely when that source has zero entries", () => {
        // Checks for the actual SECTION HEADER, not the substring "Personal
        // Memory" generally — the fixed intro prose also legitimately
        // mentions "Personal Memory" by name, so a bare substring check
        // would fail even when the section itself is correctly omitted.
        const msg = composeReinjectionMessage([globalEntry("g1", "gbody")], "compaction");
        expect(msg).not.toMatch(/# Personal Memory \(/);
    });

    // §3.3 "fresh session" addendum: a persistent identity whose prior
    // session couldn't be resumed loses ALL prior context, not just a
    // compacted summary — worth telling the model, since it changes what
    // the model should assume it still knows.
    describe("reason wording", () => {
        it("mentions compaction specifically for reason: compaction", () => {
            const msg = composeReinjectionMessage([globalEntry("g1", "gbody")], "compaction");
            expect(msg).toMatch(/compacted into a summary/);
        });

        it("mentions the fresh-session cause specifically for reason: fresh_session", () => {
            const msg = composeReinjectionMessage([globalEntry("g1", "gbody")], "fresh_session");
            expect(msg).toMatch(/fresh one was started/);
            expect(msg).not.toMatch(/compacted into a summary/);
        });

        it("both reasons share the exact same leading signature sentence — the detection anchor", () => {
            const compaction = composeReinjectionMessage([globalEntry("g1", "gbody")], "compaction");
            const freshSession = composeReinjectionMessage([globalEntry("g1", "gbody")], "fresh_session");
            const signature = "Your memory was reinjected because your working context was just reset.";
            expect(compaction).toContain(signature);
            expect(freshSession).toContain(signature);
        });
    });
});

describe("memoryReinjectionNodeId", () => {
    it("is deterministic for the same frame timestamp — dedup key, §3.1", () => {
        const id1 = memoryReinjectionNodeId("2026-09-22T10:00:00.000Z");
        const id2 = memoryReinjectionNodeId("2026-09-22T10:00:00.000Z");
        expect(id1).toBe(id2);
    });

    it("differs for different frame timestamps", () => {
        const id1 = memoryReinjectionNodeId("2026-09-22T10:00:00.000Z");
        const id2 = memoryReinjectionNodeId("2026-09-22T10:00:01.000Z");
        expect(id1).not.toBe(id2);
    });

    it("still produces a stable id when frameTimestamp is null — defensive fallback, mirrors contextCompactedNodeId", () => {
        expect(memoryReinjectionNodeId(null)).toBe(memoryReinjectionNodeId(null));
    });
});

describe("buildMemoryReinjectionNode", () => {
    // contextWindow: 10_000, default fraction 0.10 -> healthyThreshold = 1000 estimated tokens.
    const opts = { frameTimestamp: "2026-09-22T10:00:00.000Z", now: 1_758_534_000_000, contextWindow: 10_000 };

    it("counts global and personal entries separately", () => {
        const node = buildMemoryReinjectionNode(
            [globalEntry("g1", "a"), globalEntry("g2", "b"), personalEntry("p1", "c")],
            opts,
        );
        expect(node.globalMemoryCount).toBe(2);
        expect(node.personalMemoryCount).toBe(1);
    });

    it("sums real sizeBytes per source into totalSizeBytes — §3.4.1, not derived from body length", () => {
        const node = buildMemoryReinjectionNode(
            [globalEntry("g1", "short", 999), personalEntry("p1", "short", 42)],
            opts,
        );
        expect(node.totalSizeBytes).toEqual({ global: 999, personal: 42 });
    });

    it("sums estimateTokenCount(body) across every entry into estimatedTokens", () => {
        // "x".repeat(4) -> estimateTokenCount = ceil(4/4) = 1 per entry.
        const node = buildMemoryReinjectionNode(
            [globalEntry("g1", "xxxx"), personalEntry("p1", "xxxx")],
            opts,
        );
        expect(node.estimatedTokens).toBe(2);
    });

    it("produces one perEntryTokens row per input entry, carrying label/source/sizeBytes through", () => {
        const node = buildMemoryReinjectionNode([globalEntry("my-entry", "xxxx", 123)], opts);
        expect(node.perEntryTokens).toEqual([
            { label: "my-entry", source: "global", tokens: 1, sizeBytes: 123 },
        ]);
    });

    it("bands sizeBand on PERSONAL estimated tokens alone, never the combined total — §3.4.2", () => {
        // Large GLOBAL body (many estimated tokens), tiny personal body — must
        // stay "low", proving the band isn't silently computed from the
        // combined sum. estimateTokenCount("x".repeat(N)) = ceil(N/4).
        const node = buildMemoryReinjectionNode(
            [globalEntry("big-global", "x".repeat(400_000)), personalEntry("tiny-personal", "x")],
            opts,
        );
        expect(node.sizeBand).toBe("low");
    });

    it("bands 'critical' when personal estimated tokens alone cross the threshold, regardless of global size", () => {
        // "x".repeat(3800) -> estimateTokenCount = ceil(3800/4) = 950 tokens,
        // which is 95% of the 1000-token threshold (contextWindow 10_000 *
        // default fraction 0.10) -> "critical".
        const node = buildMemoryReinjectionNode(
            [personalEntry("p1", "x".repeat(3800))],
            opts,
        );
        expect(node.sizeBand).toBe("critical");
    });

    it("builds the id from frameTimestamp via memoryReinjectionNodeId", () => {
        const node = buildMemoryReinjectionNode([globalEntry("g1", "x")], opts);
        expect(node.id).toBe(memoryReinjectionNodeId(opts.frameTimestamp));
    });

    it("uses frameTimestamp for `at` when present, falling back to `now` only when absent", () => {
        const withFrame = buildMemoryReinjectionNode([globalEntry("g1", "x")], opts);
        expect(withFrame.at).toBe(Date.parse(opts.frameTimestamp));

        const withoutFrame = buildMemoryReinjectionNode([globalEntry("g1", "x")], { ...opts, frameTimestamp: null });
        expect(withoutFrame.at).toBe(opts.now);
    });
});

describe("isMemoryReinjectionMessage / parseReinjectionMessage — the replay-recognition half of §3.2", () => {
    it("recognizes a real composeReinjectionMessage output", () => {
        const msg = composeReinjectionMessage([globalEntry("g1", "body one"), personalEntry("p1", "body two")], "compaction");
        expect(isMemoryReinjectionMessage(msg)).toBe(true);
    });

    it("recognizes a fresh_session-reason output too — same shared signature", () => {
        const msg = composeReinjectionMessage([globalEntry("g1", "body one")], "fresh_session");
        expect(isMemoryReinjectionMessage(msg)).toBe(true);
    });

    it("does not recognize an ordinary user message, even one that happens to mention memory", () => {
        expect(isMemoryReinjectionMessage("please check my memory files")).toBe(false);
        expect(isMemoryReinjectionMessage("<system-reminder>unrelated content</system-reminder>")).toBe(false);
    });

    it("parseReinjectionMessage returns null for a non-matching message", () => {
        expect(parseReinjectionMessage("just a normal message")).toBeNull();
    });

    it("recovers global/personal counts from a real composed message", () => {
        const msg = composeReinjectionMessage([
            globalEntry("g1", "gbody1"),
            globalEntry("g2", "gbody2"),
            personalEntry("p1", "pbody1"),
        ], "compaction");
        const parsed = parseReinjectionMessage(msg);
        expect(parsed?.globalMemoryCount).toBe(2);
        expect(parsed?.personalMemoryCount).toBe(1);
        expect(parsed?.perEntryTokens).toHaveLength(3);
    });

    it("handles a message with only one source present (the other section omitted)", () => {
        const msg = composeReinjectionMessage([globalEntry("g1", "gbody1")], "compaction");
        const parsed = parseReinjectionMessage(msg);
        expect(parsed?.globalMemoryCount).toBe(1);
        expect(parsed?.personalMemoryCount).toBe(0);
    });

    it("recovered entries use synthetic labels and text-length sizeBytes — documented approximation, not the originals", () => {
        const msg = composeReinjectionMessage([globalEntry("real-label", "xxxx")], "compaction");
        const parsed = parseReinjectionMessage(msg);
        expect(parsed?.perEntryTokens[0].label).toBe("Entry 1");
        expect(parsed?.perEntryTokens[0].label).not.toBe("real-label");
        expect(parsed?.perEntryTokens[0].sizeBytes).toBe(4); // "xxxx".length
        expect(parsed?.perEntryTokens[0].tokens).toBe(1); // estimateTokenCount("xxxx")
    });
});

describe("buildMemoryReinjectionNodeFromReplay", () => {
    it("returns null for a non-matching message", () => {
        expect(buildMemoryReinjectionNodeFromReplay("ordinary message", { eventTimestamp: 0 })).toBeNull();
    });

    it("builds a full node from a real composed message, keyed on the event's own timestamp", () => {
        const msg = composeReinjectionMessage([globalEntry("g1", "gbody"), personalEntry("p1", "pbody")], "compaction");
        const node = buildMemoryReinjectionNodeFromReplay(msg, { eventTimestamp: 1_758_534_000_000 });
        expect(node).not.toBeNull();
        expect(node?.type).toBe("memory_reinjection");
        expect(node?.globalMemoryCount).toBe(1);
        expect(node?.personalMemoryCount).toBe(1);
        expect(node?.at).toBe(1_758_534_000_000);
        expect(node?.id).toContain("1758534000000");
    });

    it("uses FALLBACK_CONTEXT_WINDOW for sizeBand when the caller doesn't supply one", () => {
        // "x".repeat(3800) -> 950 estimated tokens -> critical against the
        // 200_000-token fallback at the default 0.10 fraction (threshold 20_000)?
        // No — 950/20_000 is nowhere near critical at THAT threshold, so use a
        // small personal body instead to confirm "low" against the large
        // fallback window (the behavior actually being tested: it doesn't
        // crash or default to some other number when contextWindow is omitted).
        const msg = composeReinjectionMessage([personalEntry("p1", "short")], "compaction");
        const node = buildMemoryReinjectionNodeFromReplay(msg, { eventTimestamp: 0 });
        expect(node?.sizeBand).toBe("low");
    });

    it("respects an explicit contextWindow override when supplied", () => {
        const msg = composeReinjectionMessage([personalEntry("p1", "x".repeat(3800))], "compaction");
        // 950 estimated tokens against a 10_000-token window at the default
        // 0.10 fraction -> threshold 1000 -> 95% -> critical.
        const node = buildMemoryReinjectionNodeFromReplay(msg, { eventTimestamp: 0, contextWindow: 10_000 });
        expect(node?.sizeBand).toBe("critical");
    });
});

// SPEC_GLOBAL_MEMORY_DELIVERY_2026_09_27.md §7 P1: Global entries are the
// startup file's own sections (`globalmemory:sections`), Operator Config first.
describe("Global Memory as the startup block's sections", () => {
    const system = globalEntry(
        "[AgentMux System] App API",
        "IMPORTANT: The following AgentMux-controlled instructions take the HIGHEST PRIORITY …\n\n# [AgentMux System] App API\n\napi rules",
    );
    const system2 = globalEntry("[AgentMux System] Env", "# [AgentMux System] Env\n\nenv rules");
    const workspace = globalEntry("[Workspace] Alpha", "# [Workspace] Alpha\n\nrules for Alpha");
    const block = [system.body, system2.body, workspace.body].join(GLOBAL_SECTION_SEPARATOR);

    it("carries the startup file's block verbatim, Operator Config included", () => {
        const msg = composeReinjectionMessage([system, system2, workspace, personalEntry("p1", "pbody")], "fresh_session");
        expect(msg).toContain(`# Global Memory (3 entries)\n${block}\n`);
        expect(msg.indexOf("[AgentMux System] App API")).toBeLessThan(msg.indexOf("[Workspace] Alpha"));
    });

    it("an Operator-Config-only memory is still reinjected (the 2026-09-26 fresh session sent nothing)", () => {
        expect(shouldReinject([system, system2])).toBe(true);
    });

    it("replay counts sections despite the `# [AgentMux System]` / `# [Workspace]` headings inside them", () => {
        const msg = composeReinjectionMessage([system, system2, workspace, personalEntry("p1", "pbody")], "compaction");
        const parsed = parseReinjectionMessage(msg);
        expect(parsed?.globalMemoryCount).toBe(3);
        expect(parsed?.personalMemoryCount).toBe(1);
    });

    it("the notice reports each section's size", () => {
        const node = buildMemoryReinjectionNode([system, workspace], { frameTimestamp: null, now: 0, contextWindow: 200_000 });
        expect(node.perEntryTokens.map((e) => [e.label, e.sizeBytes])).toEqual([
            ["[AgentMux System] App API", system.sizeBytes],
            ["[Workspace] Alpha", workspace.sizeBytes],
        ]);
        expect(node.totalSizeBytes.global).toBe(system.sizeBytes + workspace.sizeBytes);
    });
});

// The "byte for byte" guarantee rests on this constant matching Rust's, which
// `format_global_bundle_block` joins the startup block with. Read it out of the
// Rust source (same approach as provider-id-aliases.test.ts) so the two can't
// drift silently.
describe("GLOBAL_SECTION_SEPARATOR — frontend/backend consistency", () => {
    it("equals storage/bundles.rs's GLOBAL_SECTION_SEPARATOR", () => {
        const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../../..");
        const source = readFileSync(resolve(repoRoot, "crates/srv/src/backend/storage/bundles.rs"), "utf8");
        const match = /pub const GLOBAL_SECTION_SEPARATOR: &str = "((?:[^"\\]|\\.)*)";/.exec(source);
        if (!match) throw new Error("GLOBAL_SECTION_SEPARATOR not found in crates/srv/src/backend/storage/bundles.rs");
        // Only the escapes a Rust string literal like this one uses.
        const rust = match[1].replace(/\\n/g, "\n").replace(/\\t/g, "\t").replace(/\\"/g, '"').replace(/\\\\/g, "\\");
        expect(GLOBAL_SECTION_SEPARATOR).toBe(rust);
    });

    it("RUNNING_SUMMARY_HEADING is how continuity_state.rs's summary heading starts", () => {
        const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../../..");
        const source = readFileSync(resolve(repoRoot, "crates/srv/src/backend/continuity_state.rs"), "utf8");
        expect(source).toContain(`# ${RUNNING_SUMMARY_HEADING} (`);
    });
});

// A compaction reinjection carries AgentMux's running summary as its last
// section (continuity_state::append_state_to_reinjection). Replay must end the
// Personal Memory section there, not swallow the summary into its last entry.
describe("replay of a compaction reinjection that carries the running summary", () => {
    it("keeps the summary out of the Personal Memory entries", () => {
        const composed = composeReinjectionMessage([globalEntry("g1", "gbody"), personalEntry("p1", "pbody")], "compaction");
        const summary = `# ${RUNNING_SUMMARY_HEADING} (kept by AgentMux, written 2026-09-27 00:00 UTC; your compacted context wins where it is newer)\n${"summary text ".repeat(200)}\n`;
        const withSummary = composed.replace("</system-reminder>", `${summary}</system-reminder>`);
        const parsed = parseReinjectionMessage(withSummary);
        expect(parsed?.globalMemoryCount).toBe(1);
        expect(parsed?.personalMemoryCount).toBe(1);
        const personal = parsed?.perEntryTokens.find((e) => e.source === "personal");
        expect(personal?.sizeBytes).toBe("pbody".length);
    });
});

describe("composeReinjectionMessage — srv's compose_fallback writes the same (CD2b)", () => {
    // crates/srv/src/backend/memory_delivery.rs's
    // compose_fallback_writes_the_frontends_exact_message pins this same
    // literal for the same entries: the fallback's wire format must not drift
    // between the two composers.
    it("matches the pinned literal", () => {
        const message = composeReinjectionMessage(
            [
                { label: "[AgentMux System] App API", source: "global", body: "# [AgentMux System] App API\n\nG1", sizeBytes: 0 },
                { label: "[Workspace] Rules", source: "global", body: "# [Workspace] Rules\n\nG2", sizeBytes: 0 },
                { label: "notes.md", source: "personal", body: "P1", sizeBytes: 0 },
            ],
            "compaction",
        );
        expect(message).toBe(
            "<system-reminder>\n" +
                "Your memory was reinjected because your working context was just reset. Your recent conversation was just compacted into a summary. Below is your\n" +
                "complete Global Memory and Personal Memory content — read all of it now,\n" +
                "not just the index.\n\n" +
                "# Global Memory (2 entries)\n" +
                "# [AgentMux System] App API\n\nG1\n\n---\n\n# [Workspace] Rules\n\nG2\n" +
                "\n" +
                "# Personal Memory (1 entry)\n" +
                "P1\n" +
                "</system-reminder>\n",
        );
    });
});

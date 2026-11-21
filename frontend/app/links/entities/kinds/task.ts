// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Task numbers (spec §3.1). `task #N`, `work#N` (a named board), `agentmux://task/N` and a bare `#N` are tasks when
 * the task exists on a board the viewer can see. A bare `#N` that is no task is a pull request of the surface's
 * repository, when the surface has one; the explicit forms never fall back.
 */

import { openTaskEntity } from "../openers";
import type { Candidate, EntityKind, LinkContext, Resolved } from "../types";
import { pullDestination } from "./pull";

const MIN_BARE_PULL = 100;

type Form = "scheme" | "board" | "word" | "bare";

const SCHEME = /agentmux:\/\/task\/(\d{1,9})\/?/gi;
const BOARD = /(?<![\w/.#&-])([a-z][a-z0-9-]{0,23})#(\d{1,7})(?![\w-])/g;
const WORD = /(?<![\w/#-])task\s+#?(\d{1,7})(?![\w-])/gi;
const BARE = /(?<![\w&#/.-])#(\d{1,7})(?![\w-])/g;

function find(text: string, pattern: RegExp, form: Form, numberAt: number, into: Candidate[]): void {
    for (const m of text.matchAll(pattern)) {
        const data: Record<string, string> = { form, number: m[numberAt] };
        if (form === "board") data.board = m[1];
        // `task 5` and `agentmux://task/5` say what they are; `x#5` and `#5` might be something else (a name, a pull request).
        const claims = form === "scheme" || form === "word";
        into.push({ kind: "task", start: m.index, end: m.index + m[0].length, data, claims });
    }
}

function taskName(number: number): string {
    return `Open task ${number}`;
}

function resolveBare(number: number, ctx: LinkContext): Resolved | null {
    const board = ctx.board ?? ctx.tasks.defaultBoard;
    if (ctx.tasks.get(board, number)) {
        return { destination: { kind: "task", number, board: board === ctx.tasks.defaultBoard ? undefined : board }, state: "ready", name: taskName(number) };
    }
    // A short bare number is a list ordinal ("step #3"), never a pull request; the repository's own numbers have three digits or more.
    const pull = number >= MIN_BARE_PULL ? pullDestination(ctx.repo, number, false) : null;
    return pull ? { kind: "pull", destination: pull, state: "ready", name: `Open pull request ${number}` } : null;
}

export const taskKind: EntityKind = {
    id: "task",
    priority: 30,
    recognise(text) {
        const found: Candidate[] = [];
        find(text, SCHEME, "scheme", 1, found);
        find(text, BOARD, "board", 2, found);
        find(text, WORD, "word", 1, found);
        find(text, BARE, "bare", 1, found);
        return found;
    },
    resolve(candidate, ctx) {
        const number = Number(candidate.data.number);
        if (!Number.isSafeInteger(number) || number < 1) return null;
        const form = candidate.data.form as Form;
        if (form === "bare") return resolveBare(number, ctx);
        const board = form === "board" ? candidate.data.board : ctx.board ?? ctx.tasks.defaultBoard;
        if (form === "board" && !ctx.tasks.hasBoard(board)) return null;
        if (!ctx.tasks.get(board, number)) return null;
        const onDefault = board === ctx.tasks.defaultBoard;
        return { destination: { kind: "task", number, board: onDefault ? undefined : board }, state: "ready", name: taskName(number) };
    },
    open(destination, options) {
        if (destination.kind !== "task") return Promise.resolve({ ok: false, reason: "not a task" });
        return openTaskEntity(destination, options);
    },
};

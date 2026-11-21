// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pull requests and issues (spec §3.1). The repository always comes from the surface (`ctx.repo`), never from the
 * text, except in the full `owner/repo#N` form, which names its repository on purpose and always points at github.com.
 */

import { openUrlEntity } from "../openers";
import type { Candidate, Destination, EntityKind } from "../types";

// `PR` is written in capitals; the other words need a `#` or three digits, so "pull 3 changes" and "issue 3 items" are prose.
const WORD_FORM = /(?<![\w/#-])(?:(PR)[ \t]*#?(\d{1,7})|((?:[Pp]ull request|[Pp]ull|[Ii]ssue))(?:[ \t]*#(\d{1,7})|[ \t]+(\d{3,7})))(?![\w-])/g;
const FULL_FORM = /(?<![\w/.-])([A-Za-z0-9_.-]{1,39})\/([A-Za-z0-9_.-]{1,100})#(\d{1,7})(?![\w-])/g;
const REPO = /^[A-Za-z0-9_.-]{1,39}\/[A-Za-z0-9_.-]{1,100}$/;

export function pullDestination(repo: string | undefined, number: number, issue: boolean): Destination | null {
    if (!repo || !REPO.test(repo) || !Number.isSafeInteger(number) || number < 1) return null;
    return { kind: "pull", repo, number, issue };
}

export function pullUrl(dest: Extract<Destination, { kind: "pull" }>): string {
    return `https://github.com/${dest.repo}/${dest.issue ? "issues" : "pull"}/${dest.number}`;
}

export const pullKind: EntityKind = {
    id: "pull",
    priority: 35,
    recognise(text) {
        const found: Candidate[] = [];
        for (const m of text.matchAll(WORD_FORM)) {
            const issue = m[3]?.toLowerCase() === "issue";
            const number = m[2] ?? m[4] ?? m[5];
            found.push({ kind: "pull", start: m.index, end: m.index + m[0].length, data: { number, issue: String(issue) }, claims: true });
        }
        for (const m of text.matchAll(FULL_FORM)) {
            found.push({ kind: "pull", start: m.index, end: m.index + m[0].length, data: { number: m[3], repo: `${m[1]}/${m[2]}`, issue: "false" }, claims: true });
        }
        return found;
    },
    resolve(candidate, ctx) {
        const number = Number(candidate.data.number);
        const issue = candidate.data.issue === "true";
        const destination = pullDestination(candidate.data.repo ?? ctx.repo, number, issue);
        if (!destination) return null;
        return { destination, state: "ready", name: `Open ${issue ? "issue" : "pull request"} ${number}` };
    },
    open(destination, options) {
        if (destination.kind !== "pull") return Promise.resolve({ ok: false, reason: "not a pull request" });
        return openUrlEntity(pullUrl(destination), options);
    },
};

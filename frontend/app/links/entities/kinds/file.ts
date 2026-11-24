// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Files (spec §3, §5, §6.4): a name with a known extension, a relative path, an absolute path. Only a surface that
 * names the agent's pane has an author whose folders srv can check, so any other text makes no file link. A file
 * link is drawn `unchecked` and srv is asked about it lazily, on hover or focus (`file-check-store.ts`): found, or a
 * muted word with the reason. The recogniser never reads the disk and never builds a path of its own.
 */

import { siteOf } from "../bare-domain";
import { openFileEntity } from "../openers";
import type { Candidate, EntityKind } from "../types";

const EXTENSIONS =
    "md|mdx|txt|json|jsonl|ts|tsx|js|jsx|mjs|cjs|rs|toml|yaml|yml|py|go|java|kt|swift|c|h|cc|cpp|hpp|cs|rb|php|sh|ps1|bat|css|scss|html|htm|xml|csv|tsv|log|sql|lock|ini|cfg|conf|env|png|jpg|jpeg|gif|webp|svg|pdf|mp4|webm|wav|zip|tgz|patch|diff";

const NAME = "[\\w.+-]+";
const LINE = "(?::(\\d{1,6})(?::\\d{1,6})?)?";
// A drive path, an absolute path, a relative path with a folder, and a bare name with a known extension.
const PATH_FORMS = [
    `[A-Za-z]:[\\\\/](?:${NAME}[\\\\/])*${NAME}\\.[A-Za-z][A-Za-z0-9]{0,7}`,
    `/(?:${NAME}/)*${NAME}\\.[A-Za-z][A-Za-z0-9]{0,7}`,
    `(?:\\.{1,2}/)?(?:${NAME}/)+${NAME}\\.[A-Za-z][A-Za-z0-9]{0,7}`,
    `${NAME}\\.(?:${EXTENSIONS})`,
];
const FILE = new RegExp(`(?<![\\w/\\\\.:@+-])(${PATH_FORMS.join("|")})${LINE}(?![\\w/-]|\\.\\w)`, "g");

/** A first segment with a dot inside it (a host name) makes the whole an address with a path, not a folder path. */
function readsAsHost(path: string): boolean {
    const first = path.split("/")[0];
    return path.includes("/") && first !== "." && first !== ".." && !/^[A-Za-z]:/.test(first) && first.indexOf(".", 1) > 0;
}

const WHOLE = new RegExp(`^(${PATH_FORMS.join("|")})${LINE}$`);

/** Whether `raw` is, whole, what the file kind reads as a file: the web linker leaves such a word for it, so a name
 *  like a bare `.md` name (which reads as a domain) is not made a web address first. */
export function looksLikeFile(raw: string): boolean {
    const m = WHOLE.exec(raw);
    return m !== null && !readsAsHost(m[1]);
}

export const fileKind: EntityKind = {
    id: "file",
    priority: 45,
    recognise(text, ctx) {
        // No pane to ask about: no file links. Text from another computer is not vouched for here.
        if (ctx.author.kind !== "agent" || ctx.author.remote) return [];
        const found: Candidate[] = [];
        for (const m of text.matchAll(FILE)) {
            if (readsAsHost(m[1])) continue;
            found.push({ kind: "file", start: m.index, end: m.index + m[0].length, data: { path: m[1] } });
        }
        return found;
    },
    resolve(candidate, ctx) {
        if (ctx.author.kind !== "agent" || ctx.author.remote) return null;
        const path = candidate.data.path;
        return {
            destination: { kind: "file", path, blockId: ctx.author.id, confined: ctx.author.confined },
            state: "unchecked",
            name: siteOf(path) ? `Open ${path}` : `Open file ${path}`,
        };
    },
    open(destination, options) {
        if (destination.kind !== "file") return Promise.resolve({ ok: false, reason: "not a file" });
        return openFileEntity(destination, options);
    },
};

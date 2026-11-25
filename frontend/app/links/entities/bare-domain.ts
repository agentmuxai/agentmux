// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A bare name that is a file name and may also be a web site: one plain word with an ending that source files share
 * with a country (`rs`, `sh`, `py`, `pl`, `ml`). A wrong link is worse than none, and a word that happens to end that
 * way must not send the person to a stranger's site, so:
 *  - a short list of well-known sites is the file when the agent has such a file and the site when it has not;
 *  - a name written with `www.` is a web address, never a file;
 *  - any other such name is a file when the agent has it, and plain text when it has not.
 * A name that reads as a file (upper case, a usual file stem) is only ever a file, and so is a `.md` name.
 */

import { isLikelyFilename } from "@/app/element/linkify-config";

const ENDINGS = "rs|sh|py|pl|ml";
const BARE = new RegExp(`^[A-Za-z0-9][\\w-]*\\.(?:${ENDINGS})$`, "i");
const WWW = new RegExp(`^www\\.[A-Za-z0-9-]+(?:\\.[A-Za-z0-9-]+)*\\.(?:${ENDINGS})$`, "i");

/** The well-known sites with those endings. Kept short on purpose: each one is a name a message really means as the site. */
const KNOWN_SITES = ["docs.rs", "lib.rs", "rustup.rs", "bun.sh", "astral.sh"];

/** The site `written` is when no such file is there: one of the well-known sites, else null. */
export function siteOf(written: string): string | null {
    return KNOWN_SITES.includes(written.toLowerCase()) ? `https://${written}` : null;
}

/** Whether `written` is a bare name that is text, not a link, when the agent has no such file. */
export function plainWhenMissing(written: string): boolean {
    return BARE.test(written) && siteOf(written) === null && !isLikelyFilename("", written);
}

/** Whether `written` is a web address by its `www.`, whatever its ending. */
export function isWebName(written: string): boolean {
    return WWW.test(written);
}

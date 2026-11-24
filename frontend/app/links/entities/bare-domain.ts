// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A bare name that is a file name and also a web site (a short name with a country's ending, like a documentation host): a name ending in one of the countries'
 * endings that source files share, written as one plain word. It is the file when the agent has such a file in its
 * folders, and the site when it has not. A name that reads as a file (an upper-case name, `main.rs`, a name with an
 * underscore or a hyphen) is only ever a file, and so is a `.md` name: those are not domains in an agent's message.
 */

import { isLikelyFilename } from "@/app/element/linkify-config";

const AMBIGUOUS = /^(?:www\.)?[A-Za-z0-9]+\.(?:rs|sh|py|pl|ml)$/;

/** The site `written` stands for when it may be one, else null. */
export function siteOf(written: string): string | null {
    if (!AMBIGUOUS.test(written) || isLikelyFilename("", written)) return null;
    return `https://${written}`;
}

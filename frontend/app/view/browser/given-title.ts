// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { pageOrigin } from "./bypass";

/** Meta key: the address a browser pane was opened at, written with a title the opener gave it (`frame:title`). */
export const TITLE_GIVEN_AT_META = "frame:title_url";

/**
 * What to write with a new page address so the pane's title follows its site: a title the opener gave stays
 * while the page is on the site it was given for, and goes (with its marker) once the pane is on another
 * site, so the page's own title shows. A page with no site (`about:blank`, an error page's data address) is
 * not a move.
 */
export function titleMetaFor(meta: MetaType | undefined, url: string): MetaType {
    const givenAt = meta?.[TITLE_GIVEN_AT_META];
    if (typeof givenAt !== "string" || givenAt === "") return {};
    const was = pageOrigin(givenAt);
    const now = pageOrigin(url);
    if (was === undefined || now === undefined || was === now) return {};
    return { "frame:title": null, [TITLE_GIVEN_AT_META]: null } as MetaType;
}

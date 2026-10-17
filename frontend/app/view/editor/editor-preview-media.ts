// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The markdown preview's local images: relative to the document's folder on
 * this computer (SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.4.1).
 * A document on an SSH host has its images there too, and the preview reads
 * only this computer's files, so it shows none rather than a same-named local
 * file. A scratch tab's file is always on this computer.
 */

import { dirnameOf } from "@/app/element/local-media";
import type { MarkdownMediaOpts } from "@/app/element/markdown-media";

export function previewMedia(filePath: string, connection: string, isScratch: boolean): MarkdownMediaOpts | undefined {
    return connection && !isScratch ? undefined : { baseDir: dirnameOf(filePath) };
}

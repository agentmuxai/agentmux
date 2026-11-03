// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The start of a Claude Code Read line: its number (right-aligned or not),
 *  then a tab or `→`. Shared with tool-meta/file-range.ts. */
export const READ_LINE_NUMBER_RE = /^\s*(\d+)(?:\t|→)/;

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the pane's hooks reach for before it exists. A hook early in the pane's setup hands a
 * callback that runs later, from an event or an effect, and that callback needs something created
 * further down: the commands, the retry, the root element, the list's scroller. The pane fills
 * each in as it is made, and every callback reads it when it runs, as it read a later `const` or
 * `let` before the pane was split into hook groups.
 */

import type { useAgentCommands } from "./hooks/useAgentCommands";

export interface PaneLate {
    commands: ReturnType<typeof useAgentCommands>;
    retryLastTurn: () => void;
    declareAuthHealthy: () => void;
    /** The root element, assigned during render; read by the PTY width, file drop and launch term-size hooks. */
    root: HTMLDivElement | undefined;
    /** The list's scrollToBottom, assigned once AgentDocumentView mounts: the composer's onTyping and useAgentCommands' onSent call it. */
    scrollToBottom: ((reason?: string) => void) | null;
    /** AgentFooter supplies the composer's emptiness once it mounts; until then it counts as empty. */
    composerIsEmpty: (() => boolean) | null;
}

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Key prefix, in the waiting-for-you registry (waiting-for-you.ts), for a
 *  call to action that belongs to no pane. Kept apart so the sound service
 *  and the OS bridge can read it without the registry's imports. */
export const APP_KEY_PREFIX = "app:";

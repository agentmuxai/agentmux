// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// What a Browser hand-off or approval is to the lists of who waits on the person: its words, and whether
// the person left such waits out. On its own so the registry of this window (waiting-for-you.ts) and the
// list joined with srv's rows (attention-list.ts) say the same.

import { getSettingsKeyAtom } from "@/app/store/global";

/** What a Browser hand-off or approval wants of the person. */
export const BROWSER_WHAT = "needs you in the browser";

/** Whether Browser hand-offs and approvals count as waiting for the person (Settings → Sounds). */
export const browserWaitsShown = (): boolean => getSettingsKeyAtom("notify:waiting:browser")() !== false;

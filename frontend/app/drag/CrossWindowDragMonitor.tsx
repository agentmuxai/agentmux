// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Platform barrel — re-exports the correct implementation for the current build target.
// Import from this file directly (e.g. `import { setCurrentDragPayload } from "@/app/drag/CrossWindowDragMonitor"`)
// and Vite's platformResolve plugin will pick the right .win32/.darwin/.linux variant.
export { CrossWindowDragMonitor, setCurrentDragPayload, getCurrentDragPayload } from "./CrossWindowDragMonitor.platform";
export type { DragItemPayload } from "./CrossWindowDragMonitor.platform";

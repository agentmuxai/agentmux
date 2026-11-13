// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Vitest test environment setup. Wires:
 *   - `@testing-library/jest-dom` matchers (toBeInTheDocument,
 *     toBeDisabled, etc.) for component tests.
 *
 * Per-suite RPC mocks are NOT installed here — each integration
 * test uses `vi.mock("@/app/store/rpc-api", ...)` at the file level
 * with its own response fixtures.
 *
 * Spec: `docs/specs/SPEC_LAUNCH_MODAL_INTEGRATION_TESTS_2026_05_19.md`.
 */

import "@testing-library/jest-dom/vitest";
import { vi } from "vitest";

// A closed dialog leaves a stand-in backdrop in the page for 120 ms while it fades (element/modal-exit-fade.ts).
// Tests that snapshot the body or query `.modal-backdrop` after a close would see it, so it is off here;
// modal-exit-fade.test.ts and modal-exit-fade.modal.test.tsx turn it back on.
vi.mock("../frontend/app/element/modal-exit-fade", () => ({ fadeOutBackdrop: () => {} }));

// jsdom has no matchMedia, and uPlot calls it when it loads (to follow the
// pixel ratio), so any test that imports the views would fail at import.
if (typeof window.matchMedia !== "function") {
    window.matchMedia = (query: string) =>
        ({
            matches: false,
            media: query,
            onchange: null,
            addEventListener: () => {},
            removeEventListener: () => {},
            addListener: () => {},
            removeListener: () => {},
            dispatchEvent: () => false,
        }) as MediaQueryList;
}

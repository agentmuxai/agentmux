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

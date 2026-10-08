// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * How long the pane waits for an ambient pull RPC (the session title, the
 * next-message suggestion). srv lets the model call itself run for 30 s
 * (`ambient::purpose`, interactive class), after it may have queued behind other
 * calls; the RPC has to outlast both, or a reply that was paid for is dropped
 * on arrival. docs/reports/REPORT_AMBIENT_FRAMEWORK_REASSESSMENT_2026_10_08.md
 * section 6.4.
 */
export const AMBIENT_PULL_TIMEOUT_MS = 45_000;

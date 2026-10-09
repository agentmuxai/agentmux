// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * How long the pane waits for an ambient pull RPC (the session title, the
 * next-message suggestion). srv lets an interactive call wait at most 10 s for a
 * permit (`ambient::limits`) and run at most 30 s (`ambient::purpose`); the RPC
 * outlasts both, so a reply that was paid for is never dropped on arrival. docs/reports/REPORT_AMBIENT_FRAMEWORK_REASSESSMENT_2026_10_08.md
 * section 6.4.
 */
export const AMBIENT_PULL_TIMEOUT_MS = 45_000;

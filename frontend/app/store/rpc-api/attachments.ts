// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Attachments in the agent composer: hand paths to the backend for
// processing, cancel a batch, look up processed attachments, copy one into
// a container pane's working folder. See
// docs/specs/SPEC_AGENT_PANE_IMAGE_ATTACHMENTS_2026_09_26.md §6 and
// crates/srv/src/server/app_api/attachments.rs. Progress arrives as
// `attachment:*` events scoped to the pane's block.

// Request/response types are generated from the Rust structs by ts-rs.
export const AttachmentsApi = {};

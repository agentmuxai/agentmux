// Copyright 2024, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { JektDeliveryTier, JektTier } from "./node-types";

/**
 * Agent message icon mapping
 */
export const AGENT_MESSAGE_ICONS: Record<string, string> = {
    mux: "📨", // Async mailbox
    ject: "⚡", // Terminal injection
};

/**
 * Direction icon mapping
 */
export const DIRECTION_ICONS: Record<string, string> = {
    incoming: "📥",
    outgoing: "📤",
};

/**
 * Jekt tier (sensitivity) icon mapping
 */
export const JEKT_TIER_ICONS: Record<JektTier, string> = {
    info: "ℹ️",
    coord: "🔗",
    sensitive: "⚠️",
};

/**
 * Jekt delivery-tier icon mapping
 */
export const JEKT_DELIVERY_ICONS: Record<JektDeliveryTier, string> = {
    host: "🏠",
    channel: "🖥️",
    lan: "🛰️",
    wan: "☁️",
};

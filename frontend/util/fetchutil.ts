// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Utility to abstract the fetch function.
// Uses the standard fetch API.

export function fetch(input: string | Request | URL, init?: RequestInit): Promise<Response> {
    // Always use globalThis.fetch (standard Web API)
    // Chromium provides the fetch API in the renderer
    return globalThis.fetch(input, init);
}

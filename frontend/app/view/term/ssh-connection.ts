// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** A connection name that is an SSH destination: not local, not WSL. */
export function isSshConnection(conn: unknown): boolean {
    return typeof conn === "string" && conn.trim() !== "" && conn.trim() !== "local" && !conn.startsWith("wsl://");
}

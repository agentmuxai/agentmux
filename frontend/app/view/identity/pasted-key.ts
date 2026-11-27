// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** An account whose pasted key srv keeps (in the Vault, or for older accounts the OS keychain): the form shows it masked. */
export function keepsPastedKey(a: { secret_ref?: { backend?: string } | null }): boolean {
    const backend = a.secret_ref?.backend;
    return backend === "keychain" || backend === "vault";
}

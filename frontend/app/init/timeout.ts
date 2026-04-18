// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Wrap a promise with a timeout. Rejects with a descriptive error if it takes too long. */
export function withTimeout<T>(promise: Promise<T>, ms: number, label: string): Promise<T> {
    return Promise.race([
        promise,
        new Promise<T>((_, reject) =>
            setTimeout(() => reject(new Error(`Timeout: ${label} did not respond within ${ms / 1000}s`)), ms)
        ),
    ]);
}

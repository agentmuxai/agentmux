// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * How the AgentMux Cloud session reads in the UI, from `muxbus.status`. Pure
 * and dependency-free so every surface (host popover, instance panel, Armory
 * accounts) agrees, and tests can use it without mocking the controller.
 */

/** The parts of `muxbus.status` these helpers read. */
export interface MuxBusSessionFields {
    connected: boolean;
    valid: boolean;
    needsReauth?: boolean;
}

/**
 * The stored sign-in can't work anymore (its refresh was refused, or the
 * cloud moved to another sign-in client), so the UI offers "Sign in again".
 * docs/specs/SPEC_CLOUD_SETTINGS_DISCOVERY_2026_09_27.md §3.3, §3.5.
 */
export function muxbusNeedsSignInAgain(s: MuxBusSessionFields | null | undefined): boolean {
    return !!s?.needsReauth;
}

/** Signed in, unexpired, and not a sign-in the cloud has left behind. */
export function isMuxBusSessionOk(s: MuxBusSessionFields | null | undefined): boolean {
    return !!s && s.connected && s.valid && !s.needsReauth;
}

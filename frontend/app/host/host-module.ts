// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A host's startup hooks. `frontend/bootstrap.ts` imports the build's host
// module as `@host-module`: the desktop (CEF) host by default, or another
// module chosen at build time (docs/specs/SPEC_EXTERNAL_HOST_BUILD_2026_10_09.md).

export interface HostModule {
    /** Runs before any other startup code, e.g. to route console output and
     *  uncaught errors to the host's log. */
    early(): void;
    /** A frame has been painted (two animation frames after load). `label` is
     *  this window's label, `"main"` unless the URL names another. */
    firstPaint(label: string): void;
    /** Install `window.api`. `getApi()` is usable once this resolves. */
    setup(): Promise<void>;
}

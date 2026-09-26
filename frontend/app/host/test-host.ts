// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A stand-in host for tests: an `AppApi` whose every method is a harmless
// no-op unless the test overrides it, reporting `NO_HOST_CAPS` by default.
// Lets a test run UI against "a host that can't do desktop things" without
// hand-building all ~110 methods.
// docs/specs/SPEC_HOST_API_SEAM_2026_09_26.md

import { NO_HOST_CAPS } from "@/app/host/host-caps";

export function makeTestHostApi(overrides: Partial<AppApi> = {}, caps: Partial<HostCaps> = {}): AppApi {
    const base: Partial<AppApi> = {
        getHostCaps: () => ({ ...NO_HOST_CAPS, ...caps }),
        ...overrides,
    };
    return new Proxy(base, {
        get(target, prop) {
            if (prop in target) return target[prop as keyof AppApi];
            // Anything not overridden is a no-op returning undefined, so a
            // caller that awaits it continues instead of hanging.
            return () => undefined;
        },
    }) as AppApi;
}

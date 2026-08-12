// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Warden — Internet section. Closed-by-default stub, lifted verbatim out of
// the original monolithic warden.tsx. Cross-network governance ships behind
// lan-awareness Phase 4 (cloud fallback) — see
// specs/SPEC_WARDEN_WIDGET_2026-05-25.md.

import type { JSX } from "solid-js";

import "@/app/view/warden-shared/warden-manager-chrome.scss";

export const WardenInternetStub = (): JSX.Element => (
    <div class="warden-manager-body">
        <div class="warden-section-stub">
            Closed by default. Cross-network governance ships behind lan-awareness
            Phase 4 (cloud fallback).
        </div>
    </div>
);

WardenInternetStub.displayName = "WardenInternetStub";

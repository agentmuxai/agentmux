// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The event subscriptions whose handlers reach up into modals and the other store modules that read
// `global.ts` themselves. They live here, not in `initGlobalEventSubs`, so `global.ts` imports nothing
// above itself (the import-cycle ratchet, scripts/RATCHETS.md). `app-init.ts` calls both.

import { UserInputModal } from "@/app/modals/userinputmodal";
import { WpsEvent } from "@/app/store/mps-events";
import { acceptAppUpdate, APP_UPDATE_EVENT } from "./app-update";
import { acceptConnectivity, CONNECTIVITY_EVENT } from "./connectivity";
import { openModal } from "./modalmodel";
import { muxEventSubscribe, onMuxEvent } from "./mps";
import { acceptSigninLink } from "./signin-link";

export function initUiEventSubs(initOpts: AgentMuxInitOpts) {
    onMuxEvent(WpsEvent.SigninLink, (link) => acceptSigninLink(link));
    // A service's reachability changed (srv `connectivity`).
    onMuxEvent(CONNECTIVITY_EVENT, (status) => acceptConnectivity(status));
    // A newer release, or none (srv `update_check`).
    onMuxEvent(APP_UPDATE_EVENT, (status) => acceptAppUpdate(status));
    muxEventSubscribe({
        eventType: WpsEvent.UserInput,
        handler: (event) => {
            const data: UserInputRequest = event.data;
            openModal(UserInputModal, { ...data });
        },
        scope: initOpts.windowId,
    });
}

// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Split out of agent-view.tsx (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §3.5 step 6).

import { createBottomPanels, type AgentBottomPanelsProps } from "./agent-bottom-panels-state";
import {
    ActivityDockRow,
    LoginPanels,
    PendingQueue,
    RecoveryBanners,
    RequestStack,
    SessionBanners,
    TurnStatus,
} from "./agent-bottom-panels-parts";

/**
 * Everything docked between the transcript and the composer strip: login,
 * decision and question panels, the queue, the recovery banners, the activity
 * dock, an API retry in progress, input held while the API is unreachable,
 * the working row and the session notices.
 *
 * Renders a fragment, not a wrapper: each panel stays a direct flex child of
 * `.agent-view-zoomed`, which the layout and AgentDocumentVirtualList's
 * clientHeight re-pin rely on (§3.3). The one exception is the request stack
 * (permission, question, cards), which shares one 40% cap so a tall question
 * and a card together never squeeze out the transcript
 * (SPEC_VAULT_REQUESTS_AS_CARDS_IN_THE_AGENT_PANE_2026_11_01.md §4.1); the re-pin
 * watches the transcript's own size, which the stack changes as before.
 */
export const AgentBottomPanels = (props: AgentBottomPanelsProps) => {
    const t = createBottomPanels(props);
    return (
        <>
            <LoginPanels p={props} t={t} />
            <RequestStack p={props} t={t} />
            <PendingQueue p={props} t={t} />
            <RecoveryBanners p={props} t={t} />
            <ActivityDockRow p={props} t={t} />
            <TurnStatus p={props} t={t} />
            <SessionBanners p={props} t={t} />
        </>
    );
};

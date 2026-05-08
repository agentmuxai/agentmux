// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { FaviconImg } from "@/app/view/browser/components/FaviconImg";

// noAction so the IconButton renders as a decorative icon, not a button.
export function buildBrowserHeaderIcon(faviconUrl: string, title: string): IconButtonDecl {
    return {
        elemtype: "iconbutton",
        icon: <FaviconImg src={faviconUrl} size={16} />,
        noAction: true,
        title: title || "Browser",
    };
}

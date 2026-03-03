// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { QuickTips } from "@/app/element/quicktips";
import { atom, PrimitiveAtom } from "jotai";

/**
 * HelpViewModel - Simplified help widget that shows QuickTips content
 *
 * Previously extended WebViewModel to show embedded docsite in a browser.
 * Now simplified to directly render QuickTips component (same as old tips widget).
 *
 * Rationale:
 * - No need for heavy WebViewModel + WebView infrastructure
 * - Tips content is more useful than embedded browser
 * - Faster, simpler, cleaner UX
 */
class HelpViewModel implements ViewModel {
    viewType: string;
    showTocAtom: PrimitiveAtom<boolean>;

    constructor() {
        this.viewType = "help";
        this.showTocAtom = atom(false);
    }

    get viewComponent(): ViewComponent {
        return HelpView;
    }

    showTocToggle() {
        // Optional: toggle table of contents if QuickTips supports it
        // (Currently unused, but kept for future enhancement)
    }
}

function HelpView({ model }: { model: HelpViewModel }) {
    return (
        <div className="px-[5px] py-[10px] overflow-auto w-full">
            <QuickTips />
        </div>
    );
}

export { HelpViewModel };

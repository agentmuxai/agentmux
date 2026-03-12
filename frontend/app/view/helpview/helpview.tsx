// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { QuickTips } from "@/app/element/quicktips";
import { createSignalAtom } from "@/util/util";
import type { SignalAtom } from "@/util/util";
import type { JSX } from "solid-js";

/**
 * HelpViewModel - Simplified help widget that shows QuickTips content
 */
class HelpViewModel implements ViewModel {
    viewType: string;
    showTocAtom: SignalAtom<boolean>;

    constructor() {
        this.viewType = "help";
        this.showTocAtom = createSignalAtom(false);
    }

    get viewComponent(): ViewComponent {
        return HelpView as unknown as ViewComponent;
    }

    showTocToggle() {
        // Optional: toggle table of contents if QuickTips supports it
    }
}

function HelpView({ model }: { model: HelpViewModel }): JSX.Element {
    return (
        <div class="px-[5px] py-[10px] overflow-auto w-full">
            <QuickTips />
        </div>
    );
}

export { HelpViewModel };

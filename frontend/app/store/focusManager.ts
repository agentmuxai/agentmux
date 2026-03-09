import { getBlockComponentModel } from "@/app/store/global";
import { globalStore } from "@/app/store/jotaiStore";
import { focusedBlockId } from "@/util/focusutil";
import { getLayoutModelForStaticTab } from "@/layout/index";
import { Atom, atom } from "jotai";

class FocusManager {
    blockFocusAtom: Atom<string | null>;

    constructor() {
        this.blockFocusAtom = atom((get) => {
            const layoutModel = getLayoutModelForStaticTab();
            const lnode = get(layoutModel.focusedNode);
            return lnode?.data?.blockId;
        });
    }

    setBlockFocus(force: boolean = false) {
        this.refocusNode();
    }

    nodeFocusWithin(): boolean {
        return focusedBlockId() != null;
    }

    requestNodeFocus(): void {
        // no-op, node is the only focus target now
    }

    getFocusType(): "node" {
        return "node";
    }

    refocusNode() {
        const layoutModel = getLayoutModelForStaticTab();
        const lnode = globalStore.get(layoutModel.focusedNode);
        if (lnode == null || lnode.data?.blockId == null) {
            return;
        }
        layoutModel.focusNode(lnode.id);
        const blockId = lnode.data.blockId;
        const bcm = getBlockComponentModel(blockId);
        const ok = bcm?.viewModel?.giveFocus?.();
        if (!ok) {
            const inputElem = document.getElementById(`${blockId}-dummy-focus`);
            inputElem?.focus();
        }
    }
}

export const focusManager = new FocusManager();

// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { atoms, globalStore } from "@/store/global";
import { modalsModel } from "@/store/modalmodel";
import * as jotai from "jotai";
import { useEffect } from "react";
import { getModalComponent } from "./modalregistry";

const ModalsRenderer = () => {
    const [modals] = jotai.useAtom(modalsModel.modalsAtom);
    const rtn: React.ReactElement[] = [];
    for (const modal of modals) {
        const ModalComponent = getModalComponent(modal.displayName);
        if (ModalComponent) {
            rtn.push(<ModalComponent key={modal.displayName} {...modal.props} />);
        }
    }
    // Onboarding/tutorial modals disabled for lightweight build
    useEffect(() => {
        globalStore.set(atoms.modalOpen, rtn.length > 0);
    }, [rtn]);

    return <>{rtn}</>;
};

export { ModalsRenderer };

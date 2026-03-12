// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0
//
// SolidJS migration: Jotai PrimitiveAtom → createSignal

import { createSignal } from "solid-js";

class ModalsModel {
    private _modals: () => Array<{ displayName: string; props?: any }>;
    private _setModals: (v: Array<{ displayName: string; props?: any }>) => void;

    constructor() {
        const [get, set] = createSignal<Array<{ displayName: string; props?: any }>>([]);
        this._modals = get;
        this._setModals = set;
    }

    /** Reactive accessor — call in a SolidJS component to get live modal list. */
    get modalsAtom() {
        return this._modals;
    }

    pushModal = (displayName: string, props?: any) => {
        this._setModals([...this._modals(), { displayName, props }]);
    };

    popModal = (callback?: () => void) => {
        const modals = this._modals();
        if (modals.length > 0) {
            this._setModals(modals.slice(0, -1));
            if (callback) callback();
        }
    };

    hasOpenModals(): boolean {
        return this._modals().length > 0;
    }

    isModalOpen(displayName: string): boolean {
        return this._modals().some((modal) => modal.displayName === displayName);
    }
}

const modalsModel = new ModalsModel();

export { modalsModel };

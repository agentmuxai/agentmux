// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { TypeAheadModal } from "@/app/modals/typeaheadmodal";
import { globalRefocusWithTimeout } from "@/app/store/keymodel";
import * as keyutil from "@/util/keyutil";
import { createConnPicker, type ConnModalProps } from "./conntypeahead-state";

const ChangeConnectionBlockModal = (props: ConnModalProps) => {
    const s = createConnPicker(props);

    if (!props.changeConnModalOpen()) {
        return null;
    }

    return (
        <TypeAheadModal
            blockRef={props.blockRef}
            anchorRef={props.connBtnRef}
            suggestions={s.suggestions()}
            onSelect={(selected: string) => {
                s.changeConnection(selected);
                props.setChangeConnModalOpen(false);
                globalRefocusWithTimeout(10);
            }}
            selectIndex={s.rowIndex()}
            autoFocus={s.isNodeFocused()}
            onKeyDown={(e) => keyutil.keydownWrapper(s.handleTypeAheadKeyDown)(e)}
            onChange={(current: string) => s.setConnSelected(current)}
            value={s.connSelected()}
            label="Connect to (username@host)..."
            onClickBackdrop={() => props.setChangeConnModalOpen(false)}
        />
    );
};

export { ChangeConnectionBlockModal };

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// "Pair a device" in the host popover: a QR the AgentMux Mobile app scans to
// pair with this computer's viewer (crates/srv/src/backend/viewer). The QR
// carries a one-time code and the fingerprint of this computer's certificate,
// never this instance's auth key. It works for 2 minutes; "New code" cancels
// it and shows another. Pairing needs LAN discovery on, because the device
// connects over this network.

import type { Accessor, JSX } from "solid-js";
import { PairButtons, PairCode, PairNotes } from "./PairDevicePanelParts";
import { createPairDevice } from "./PairDevicePanelState";

export { countdown } from "./PairDevicePanelState";

export function PairDevicePanel(props: { lanDiscoveryEnabled: Accessor<boolean> }): JSX.Element {
    const s = createPairDevice(props);
    return (
        <div class="status-bar-pair" data-testid="pair-device">
            <PairButtons s={s} lan={props.lanDiscoveryEnabled} />
            <PairNotes s={s} lan={props.lanDiscoveryEnabled} />
            <PairCode s={s} />
        </div>
    );
}

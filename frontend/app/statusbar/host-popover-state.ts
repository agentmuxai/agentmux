// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The host popover panel's state (HostPopover.tsx): opening the data folder and the error it can leave, the
 * file manager's name on this platform, and how the MuxBus Cloud session reads.
 */

import { getApi, type LanDiscoverability, type LanFirewall } from "@/store/global";
import { createSignal, type Accessor } from "solid-js";
import { type PopoverAnchor } from "@/app/element/anchored-popover";
import { type MuxBusController } from "@/app/view/accounts/AgentMuxConnectPanel";
import { isMuxBusSessionOk, muxbusNeedsSignInAgain } from "@/app/view/accounts/muxbus-session";
import { isLinux, isMacOS } from "@/util/platformutil";
import type { LanInstance } from "@/types/rpc/LanInstance";

export type HostInfo = {
    hostname: string;
    os: string;
    localIp: string;
    version: string;
    dataDir: string;
    pid: number;
    ports: {
        ipc: string;
        web: string;
        ws: string;
        devtools: string;
    };
};

export interface HostPopoverPanelProps {
    /** The status bar trigger. */
    anchor: PopoverAnchor;
    onClose: () => void;
    hostname: string;
    hostInfo: Accessor<HostInfo | null>;
    lanInstances: Accessor<LanInstance[]>;
    lanCount: Accessor<number>;
    lanDiscoveryEnabled: Accessor<boolean>;
    lanDiscoveryError: Accessor<string | null>;
    lanDiscoverability: Accessor<LanDiscoverability | null>;
    lanFirewall: Accessor<LanFirewall | null>;
    onLanToggle: (enabled: boolean) => void;
    muxbus: MuxBusController;
    /** `statusbar:showmuxbuscloud`: false hides the MuxBus Cloud block. Absent means shown. */
    showMuxbusCloud?: Accessor<boolean>;
}

/** The host popover panel's state: everything its markup reads beyond its props, and what its controls do. */
export function createHostPanel(props: HostPopoverPanelProps) {
    // Data-path link → OS file manager. Errors surface inline (a headless
    // Linux box with no handler would otherwise look like a dead link).
    const fileManagerLabel = () =>
        isMacOS() ? "Reveal in Finder" : isLinux() ? "Open in file manager" : "Show in File Explorer";
    const [openDirError, setOpenDirError] = createSignal<string | null>(null);
    const openDataDir = async () => {
        setOpenDirError(null);
        try {
            await getApi().openDataDirInFileManager();
        } catch (e) {
            setOpenDirError(`Couldn't open folder: ${e}`);
        }
    };

    const muxbus = props.muxbus;
    const showMuxbusCloud = () => props.showMuxbusCloud?.() ?? true;
    const muxbusOk = () => isMuxBusSessionOk(muxbus.status());
    const signInAgain = () => muxbusNeedsSignInAgain(muxbus.status());
    return { fileManagerLabel, openDirError, openDataDir, muxbus, showMuxbusCloud, muxbusOk, signInAgain };
}

/** What the host popover panel's markup reads beyond its props. */
export type HostPanelState = ReturnType<typeof createHostPanel>;

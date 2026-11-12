// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Paired devices: every device paired with this computer's viewer (the host
// popover's "Pair a device"), when it paired and was last seen, whether it may
// send messages to agents as you (off until turned on here; agentmux-mobile's
// SPEC_SEND_FROM_THE_PHONE_2026_10_13 §4), and Revoke, which stops its token
// working and closes what it is watching. Below it,
// MuxBus sign-in (muxbus-sign-in.tsx) and Cloud presence (cloud-presence.tsx).
// agentmux-mobile's SPEC_AGENT_STATUS_AND_LIVE_PANE_FEED_2026_10_07 §4.1.

import type { JSX } from "solid-js";

import { settingsAtom } from "@/app/store/global";
import { settingEntry, type SettingsIndexEntry } from "../settings-model";
import { SectionHeader, set, SettingRow, ToggleControl } from "../settings-controls";
import { CLOUD_PRESENCE_SETTING, CloudPresence } from "./cloud-presence";
import { createDevices } from "./devices-section-state";
import { PairedDevices } from "./devices-section-parts";
import { HIDDEN_AGENTS_SETTING, HiddenAgents } from "./hidden-agents";
import { MUXBUS_SIGN_IN_SETTING, MuxBusSignIn } from "./muxbus-sign-in";
import { PINNED_SENDERS_SETTING, PinnedSenders } from "./pinned-senders";

export { ago } from "./devices-section-state";

// ── Search index — see appearance-section.tsx's header comment for the pattern. ──

export const DEVICES_SETTINGS = {
    pairedDevices: settingEntry({
        id: "devices.paired",
        labelKey: "settings.devices.pairedDevices.label",
        descriptionKey: "settings.devices.pairedDevices.description",
        section: "devices",
        keywords: ["mobile", "device", "tablet", "pair", "revoke", "viewer", "live feed", "qr code", "send", "message"],
    }),
    shareTerminals: settingEntry({
        id: "devices.share_terminals",
        labelKey: "settings.devices.shareTerminals.label",
        descriptionKey: "settings.devices.shareTerminals.description",
        section: "devices",
        groupKey: "settings.devices.group.pairedDevices",
        keywords: ["terminal", "shell", "mobile", "device", "share", "watch", "viewer", "viewer:shareterminals"],
    }),
    hiddenAgents: HIDDEN_AGENTS_SETTING,
    muxbus: MUXBUS_SIGN_IN_SETTING,
    cloudPresence: CLOUD_PRESENCE_SETTING,
    pinnedSenders: PINNED_SENDERS_SETTING,
    publishPresence: settingEntry({
        id: "devices.publish_presence",
        labelKey: "settings.devices.publishPresence.label",
        descriptionKey: "settings.devices.publishPresence.description",
        section: "devices",
        keywords: ["cloud", "presence", "publish", "offline", "mobile", "devices", "cloud:publishpresence"],
    }),
    shareSummaries: settingEntry({
        id: "devices.share_summaries",
        labelKey: "settings.devices.shareSummaries.label",
        descriptionKey: "settings.devices.shareSummaries.description",
        section: "devices",
        keywords: ["swarm", "summary", "share", "lan", "network", "channel", "status", "swarm:sharesummaries"],
    }),
    statusbarMuxbusCloud: settingEntry({
        id: "devices.statusbar_muxbus_cloud",
        labelKey: "settings.devices.statusbarMuxbusCloud.label",
        descriptionKey: "settings.devices.statusbarMuxbusCloud.description",
        section: "devices",
        keywords: ["muxbus", "cloud", "dot", "status bar", "statusbar", "hide", "sign in", "statusbar:showmuxbuscloud"],
    }),
} satisfies Record<string, SettingsIndexEntry>;

export function DevicesSection(): JSX.Element {
    const s = createDevices();

    return (
        <div class="settings-section-body">
            <SectionHeader label={DEVICES_SETTINGS.pairedDevices.label} />
            <PairedDevices
                s={s}
                id={DEVICES_SETTINGS.pairedDevices.id}
                description={DEVICES_SETTINGS.pairedDevices.description}
            />
            <SettingRow
                id={DEVICES_SETTINGS.shareTerminals.id}
                label={DEVICES_SETTINGS.shareTerminals.label}
                description={DEVICES_SETTINGS.shareTerminals.description}
                control={
                    <ToggleControl
                        checked={!!settingsAtom()?.["viewer:shareterminals"]}
                        // Off is the absent key, not false: off by default.
                        onChange={(v) => set("viewer:shareterminals", v ? true : null)}
                    />
                }
            />
            <SectionHeader label={DEVICES_SETTINGS.hiddenAgents.label} />
            <HiddenAgents />
            <SectionHeader label={DEVICES_SETTINGS.muxbus.label} />
            <MuxBusSignIn />
            <SectionHeader label={DEVICES_SETTINGS.cloudPresence.label} />
            <CloudPresence />
            <SectionHeader label={DEVICES_SETTINGS.pinnedSenders.label} />
            <PinnedSenders />
            <SettingRow
                id={DEVICES_SETTINGS.publishPresence.id}
                label={DEVICES_SETTINGS.publishPresence.label}
                description={DEVICES_SETTINGS.publishPresence.description}
                control={
                    <ToggleControl
                        checked={settingsAtom()?.["cloud:publishpresence"] !== false}
                        onChange={(v) => set("cloud:publishpresence", v)}
                    />
                }
            />
            <SettingRow
                id={DEVICES_SETTINGS.shareSummaries.id}
                label={DEVICES_SETTINGS.shareSummaries.label}
                description={DEVICES_SETTINGS.shareSummaries.description}
                control={
                    <ToggleControl
                        checked={settingsAtom()?.["swarm:sharesummaries"] !== false}
                        onChange={(v) => set("swarm:sharesummaries", v)}
                    />
                }
            />
            <SettingRow
                id={DEVICES_SETTINGS.statusbarMuxbusCloud.id}
                label={DEVICES_SETTINGS.statusbarMuxbusCloud.label}
                description={DEVICES_SETTINGS.statusbarMuxbusCloud.description}
                control={
                    <ToggleControl
                        checked={settingsAtom()?.["statusbar:showmuxbuscloud"] !== false}
                        onChange={(v) => set("statusbar:showmuxbuscloud", v)}
                    />
                }
            />
        </div>
    );
}

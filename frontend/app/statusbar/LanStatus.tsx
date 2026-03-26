// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { modalsModel } from "@/app/store/modalmodel";
import { lanInstancesAtom } from "@/store/global";
import { For, Show, type JSX } from "solid-js";

const LanInstancesModal = ({ instances }: { instances: LanInstance[] }): JSX.Element => (
    <div class="config-error-message">
        <h3>LAN Instances</h3>
        <ul style={{ "list-style": "none", padding: "0", margin: "0" }}>
            <For each={instances}>
                {(inst) => (
                    <li style={{ padding: "4px 0", display: "flex", gap: "8px", "align-items": "center" }}>
                        <span style={{ color: "var(--accent-color)" }}>{"◆"}</span>
                        <span>{inst.hostname || inst.instance_id}</span>
                        <span style={{ opacity: "0.5", "font-size": "0.9em" }}>
                            v{inst.version}
                        </span>
                        <span style={{ opacity: "0.4", "font-size": "0.85em" }}>
                            {inst.address}:{inst.port}
                        </span>
                    </li>
                )}
            </For>
        </ul>
    </div>
);

const LanStatus = (): JSX.Element => {
    const count = () => lanInstancesAtom().length;

    const handleClick = () => {
        modalsModel.pushModal("MessageModal", {
            children: <LanInstancesModal instances={lanInstancesAtom()} />,
        });
    };

    return (
        <Show when={count() > 0}>
            <div
                class="status-bar-item clickable"
                title={`${count()} other AgentMux instance${count() !== 1 ? "s" : ""} on LAN`}
                onClick={handleClick}
            >
                <span class="status-icon" style={{ color: "var(--accent-color)" }}>
                    {"◆"}
                </span>
                <span>{count()} on LAN</span>
            </div>
        </Show>
    );
};

LanStatus.displayName = "LanStatus";

export { LanStatus };

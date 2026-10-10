// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Settings → Browser: the named browser profiles — rename, recolour, delete,
// add (docs/specs/SPEC_BROWSER_PANE_PROFILES_MENU_2026_10_09.md §3).

import { createSignal, For, onMount, Show, type JSX } from "solid-js";
import clsx from "clsx";
import { Button, TextInput } from "@/app/element/ui";
import { getPlatform } from "@/util/platformutil";
import type { BrowserProfile } from "@/types/rpc/BrowserProfile";
import {
    browserProfiles,
    createBrowserProfile,
    deleteBrowserProfile,
    loadBrowserProfiles,
    PROFILE_COLORS,
    updateBrowserProfile,
} from "@/app/view/browser/browser-profiles";
import { ProfileBadge } from "@/app/view/browser/browser-profile-menu";
import type { SettingsIndexEntry } from "../settings-model";
import { SectionHeader } from "../settings-controls";

export const BROWSER_SETTINGS = {
    profiles: {
        id: "browser.profiles",
        label: "Browser profiles",
        description:
            "Each profile keeps its own sign-ins, cookies and site data, so one site can be open as two accounts side by side. Open a tab in one from the Profile button in a browser pane. Personal is the one every tab started with; it can't be removed.",
        section: "browser",
        keywords: ["profile", "profiles", "account", "accounts", "sign in", "cookies", "incognito", "identity", "work"],
    },
} satisfies Record<string, SettingsIndexEntry>;

function ProfileRow(props: { profile: BrowserProfile }): JSX.Element {
    const [name, setName] = createSignal(props.profile.name);
    const [error, setError] = createSignal<string | null>(null);
    const [confirming, setConfirming] = createSignal(false);
    let confirmTimer: ReturnType<typeof setTimeout> | undefined;

    const run = async (f: () => Promise<void>) => {
        setError(null);
        try {
            await f();
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
    };
    const rename = () => {
        const next = name().trim();
        if (next && next !== props.profile.name) void run(() => updateBrowserProfile(props.profile.id, { name: next }));
        else setName(props.profile.name);
    };
    // Delete asks once more on the button itself: it closes the profile's
    // tabs and forgets its sign-ins for good.
    const remove = () => {
        if (!confirming()) {
            setConfirming(true);
            confirmTimer = setTimeout(() => setConfirming(false), 5000);
            return;
        }
        clearTimeout(confirmTimer);
        setConfirming(false);
        void run(() => deleteBrowserProfile(props.profile.id));
    };

    return (
        <div class="browser-profile-row">
            <ProfileBadge name={props.profile.name} color={props.profile.color} />
            <TextInput
                detached
                aria-label={`Name of ${props.profile.name}`}
                value={name()}
                maxLength={40}
                onInput={(e) => setName(e.currentTarget.value)}
                onBlur={rename}
                onKeyDown={(e) => {
                    if (e.key === "Enter") (e.currentTarget as HTMLInputElement).blur();
                    if (e.key === "Escape") setName(props.profile.name);
                }}
            />
            <div class="browser-profile-swatches" role="radiogroup" aria-label={`Colour of ${props.profile.name}`}>
                <For each={PROFILE_COLORS}>
                    {(c) => (
                        <Button
                            class={clsx("browser-profile-swatch", { selected: props.profile.color === c })}
                            style={{ "--swatch": c }}
                            role="radio"
                            aria-checked={props.profile.color === c}
                            aria-label={c}
                            onClick={() => void run(() => updateBrowserProfile(props.profile.id, { color: c }))}
                        />
                    )}
                </For>
            </div>
            <Button tone={confirming() ? "danger" : "quiet"} icon="trash" onClick={remove}>
                {confirming() ? `Delete ${props.profile.name} and sign out?` : "Delete"}
            </Button>
            <Show when={error()}>
                <div class="settings-config-error" role="alert">
                    {error()}
                </div>
            </Show>
        </div>
    );
}

export function BrowserSection(): JSX.Element {
    const [newName, setNewName] = createSignal("");
    const [error, setError] = createSignal<string | null>(null);
    onMount(() => void loadBrowserProfiles());
    const windowsOnly = getPlatform() !== "win32";

    const add = async () => {
        const name = newName().trim();
        if (!name) return;
        setError(null);
        try {
            await createBrowserProfile(name);
            setNewName("");
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
    };

    return (
        <div class="settings-section-body">
            <SectionHeader label={BROWSER_SETTINGS.profiles.label} />
            <div id={`setting-${BROWSER_SETTINGS.profiles.id}`} class="setting-row">
                <div class="setting-devices-description">{BROWSER_SETTINGS.profiles.description}</div>
                <Show when={windowsOnly}>
                    <div class="setting-devices-empty">Browser profiles are Windows only for now.</div>
                </Show>
                <div class="browser-profile-list">
                    <div class="browser-profile-row">
                        <span class="browser-profile-badge browser-profile-badge-personal" aria-hidden="true">
                            <i class="fa-solid fa-user" />
                        </span>
                        <span class="browser-profile-fixed">Personal</span>
                    </div>
                    <For each={browserProfiles()}>{(p) => <ProfileRow profile={p} />}</For>
                </div>
                <Show when={!windowsOnly}>
                    <div class="browser-profile-add">
                        <TextInput
                            detached
                            aria-label="New profile name"
                            placeholder="New profile name"
                            value={newName()}
                            maxLength={40}
                            onInput={(e) => setNewName(e.currentTarget.value)}
                            onKeyDown={(e) => {
                                if (e.key === "Enter") void add();
                            }}
                        />
                        <Button icon="plus" disabled={!newName().trim()} onClick={() => void add()}>
                            Add profile
                        </Button>
                    </div>
                </Show>
                <Show when={error()}>
                    <div class="settings-config-error" role="alert">
                        {error()}
                    </div>
                </Show>
            </div>
        </div>
    );
}

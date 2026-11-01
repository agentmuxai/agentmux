// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Settings → Browser: the named browser profiles — rename, recolour, let
// agents use one, delete, add
// (docs/specs/SPEC_BROWSER_PANE_PROFILES_MENU_2026_10_09.md §3, §5).

import { createSignal, For, onMount, Show, type JSX } from "solid-js";
import clsx from "clsx";
import { Button, IconButton, Switch, TextInput } from "@/app/element/ui";
import { normalizeKeepAwakeSite } from "@/app/view/browser/hibernation";
import type { BrowserProfile } from "@/types/rpc/BrowserProfile";
import {
    browserProfiles,
    createBrowserProfile,
    deleteBrowserProfile,
    profileSignins,
    loadBrowserProfiles,
    PROFILE_COLORS,
    updateBrowserProfile,
} from "@/app/view/browser/browser-profiles";
import { ProfileBadge } from "@/app/view/browser/browser-profile-menu";
import type { SettingsIndexEntry } from "../settings-model";
import { settingsAtom } from "@/app/store/global";
import { NumberControl, SectionHeader, set, SettingRow, ToggleControl } from "../settings-controls";
import { AGENT_SITE_GRANTS_SETTING, AgentSiteGrants } from "./agent-site-grants";
import { browserPauseAll, setAgentBrowsingPaused, startBrowserPauseAll } from "@/app/store/browser-pause-all";

export const BROWSER_SETTINGS = {
    profiles: {
        id: "browser.profiles",
        label: "Browser profiles",
        description:
            "Each profile keeps its own sign-ins, cookies and site data, so one site can be open as two accounts side by side. Open a tab in one from the Profile button in a browser pane. Personal is the one every tab started with; it can't be removed. An agent can browse as a profile only if you switch on \"Agents may use it\" for that profile; on Windows it can also use an Incognito tab.",
        section: "browser",
        keywords: ["profile", "profiles", "account", "accounts", "sign in", "cookies", "incognito", "identity", "work", "agent", "agents"],
    },
    searchSuggestions: {
        id: "browser.search_suggestions",
        label: "Search suggestions while typing",
        description:
            "The address bar also asks your search engine for suggestions as you type, as Chrome does, which sends it what you type before you press Enter. Incognito tabs never ask, as in Chrome. Turn it off to keep suggestions to this computer (your history and the addresses you typed).",
        section: "browser",
        keywords: ["autocomplete", "suggestions", "address bar", "omnibox", "google", "search", "privacy", "browser:searchsuggestions"],
    },
    hibernate: {
        id: "browser.hibernate",
        label: "Hibernate hidden browser panes",
        description:
            "A browser pane out of sight for a while goes to sleep: its page is put away to free its memory, often 100 to 200 MB, and loads again when you're about to look at it, with a note while it restores. It comes back with its back/forward history, scroll position and anything typed into it, on browser engines that support it; on older ones it loads fresh. Never a pane an agent is using, one playing or capturing media, one with something typed into it, or a local dev server.",
        section: "browser",
        keywords: ["hibernate", "sleep", "sleeping", "memory", "discard", "tabs", "browser:hibernate"],
    },
    hibernateAfter: {
        id: "browser.hibernate_after",
        label: "Minutes hidden before it sleeps",
        description: "Sooner when the computer is low on memory: 5 minutes, then 1.",
        section: "browser",
        keywords: ["hibernate", "sleep", "minutes", "delay", "browser:hibernateafter"],
    },
    keepAwakeSites: {
        id: "browser.keep_awake_sites",
        label: "Sites that never sleep",
        description:
            "Browser panes on these sites stay awake however long they are hidden. A site covers its subdomains (example.com covers mail.example.com); write an address with https:// to keep just that one.",
        section: "browser",
        keywords: ["hibernate", "sleep", "keep awake", "never", "sites", "exceptions", "browser:keepawakesites"],
    },
    pauseAll: {
        id: "browser.pause_all",
        label: "Pause all agent browsing",
        description:
            "An emergency stop: while it is on, every browser pane an agent drives refuses every call the agent makes, and the clocks of auto-approve grants stand still. It stays on after a restart until you turn it off here, in the status bar or from the command palette.",
        section: "browser",
        keywords: ["pause", "stop", "agent", "agents", "browsing", "emergency", "freeze", "resume", "auto-approve"],
    },
    agentSites: AGENT_SITE_GRANTS_SETTING,
} satisfies Record<string, SettingsIndexEntry>;

/** The keep-awake sites list: each site with a remove button, and a field to add one. */
function KeepAwakeSites(props: { sites: string[] }): JSX.Element {
    const [draft, setDraft] = createSignal("");
    const [error, setError] = createSignal<string | null>(null);
    const add = () => {
        const site = normalizeKeepAwakeSite(draft());
        if (!site) {
            setError(`"${draft().trim()}" isn't a site: write a host like example.com, or an address.`);
            return;
        }
        setError(null);
        setDraft("");
        if (!props.sites.includes(site)) set("browser:keepawakesites", [...props.sites, site]);
    };
    const remove = (site: string) => {
        const rest = props.sites.filter((s) => s !== site);
        set("browser:keepawakesites", rest.length ? rest : null);
    };
    return (
        <div class="browser-keepawake">
            <Show when={props.sites.length > 0}>
                <ul class="browser-keepawake-list" aria-label="Sites that never sleep">
                    <For each={props.sites}>
                        {(site) => (
                            <li class="browser-keepawake-site">
                                <span>{site}</span>
                                <IconButton icon="xmark" label={`Let ${site} sleep again`} density="compact" onClick={() => remove(site)} />
                            </li>
                        )}
                    </For>
                </ul>
            </Show>
            <div class="browser-profile-add">
                <TextInput
                    detached
                    aria-label="Site to keep awake"
                    placeholder="example.com"
                    value={draft()}
                    onInput={(e) => setDraft(e.currentTarget.value)}
                    onKeyDown={(e) => {
                        if (e.key === "Enter") add();
                    }}
                />
                <Button icon="plus" disabled={!draft().trim()} onClick={add}>
                    Add site
                </Button>
            </div>
            <Show when={error()}>
                <div class="settings-config-error" role="alert">
                    {error()}
                </div>
            </Show>
        </div>
    );
}

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
    // Delete asks once more on the button itself: it closes the profile's tabs
    // and signs it out. Its saved passwords may exist nowhere else, so they move
    // to Personal unless the person deletes them too (Vault spec §9.4).
    const saved = () => profileSignins(props.profile.id);
    const remove = (deleteSignins = false) => {
        if (!confirming()) {
            setConfirming(true);
            confirmTimer = setTimeout(() => setConfirming(false), saved() > 0 ? 10000 : 5000);
            return;
        }
        clearTimeout(confirmTimer);
        setConfirming(false);
        void run(() => deleteBrowserProfile(props.profile.id, deleteSignins));
    };
    const signinsWord = () => (saved() === 1 ? "1 saved sign-in" : `${saved()} saved sign-ins`);

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
            <label class="browser-profile-agents">
                <Switch
                    checked={props.profile.agents_allowed}
                    ariaLabel={`Agents may use ${props.profile.name}`}
                    onChange={(on) => void run(() => updateBrowserProfile(props.profile.id, { agents_allowed: on }))}
                />
                <span>Agents may use it</span>
            </label>
            <Show
                when={confirming() && saved() > 0}
                fallback={
                    <Button tone={confirming() ? "danger" : "quiet"} icon="trash" onClick={() => remove()}>
                        {confirming() ? `Delete ${props.profile.name} and sign out?` : "Delete"}
                    </Button>
                }
            >
                <div class="browser-profile-delete" role="group" aria-label={`Delete ${props.profile.name}: its ${signinsWord()}`}>
                    <span>Delete {props.profile.name}? It has {signinsWord()} in the Vault.</span>
                    <Button tone="danger" onClick={() => remove(false)}>
                        Move them to Personal
                    </Button>
                    <Button tone="quiet" onClick={() => remove(true)}>
                        Delete them too
                    </Button>
                </div>
            </Show>
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
    onMount(() => {
        void loadBrowserProfiles();
        startBrowserPauseAll();
    });
    const s = () => settingsAtom() ?? ({} as any);

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
            <SettingRow
                id={BROWSER_SETTINGS.searchSuggestions.id}
                label={BROWSER_SETTINGS.searchSuggestions.label}
                description={BROWSER_SETTINGS.searchSuggestions.description}
                control={
                    <ToggleControl
                        checked={s()["browser:searchsuggestions"] !== false}
                        onChange={(v) => set("browser:searchsuggestions", v)}
                    />
                }
            />
            <SettingRow
                id={BROWSER_SETTINGS.hibernate.id}
                label={BROWSER_SETTINGS.hibernate.label}
                description={BROWSER_SETTINGS.hibernate.description}
                control={
                    <ToggleControl
                        checked={s()["browser:hibernate"] !== false}
                        onChange={(v) => set("browser:hibernate", v)}
                    />
                }
            />
            <Show when={s()["browser:hibernate"] !== false}>
                <SettingRow
                    indent
                    id={BROWSER_SETTINGS.hibernateAfter.id}
                    label={BROWSER_SETTINGS.hibernateAfter.label}
                    description={BROWSER_SETTINGS.hibernateAfter.description}
                    control={
                        <NumberControl
                            min={1}
                            max={1440}
                            step={5}
                            parse="int"
                            value={s()["browser:hibernateafter"] ?? 30}
                            onChange={(v) => set("browser:hibernateafter", v)}
                        />
                    }
                />
                <SettingRow
                    indent
                    id={BROWSER_SETTINGS.keepAwakeSites.id}
                    label={BROWSER_SETTINGS.keepAwakeSites.label}
                    description={BROWSER_SETTINGS.keepAwakeSites.description}
                    control={<KeepAwakeSites sites={(Array.isArray(s()["browser:keepawakesites"]) ? s()["browser:keepawakesites"] : []).filter((x: unknown) => typeof x === "string")} />}
                />
            </Show>
            <SectionHeader label={BROWSER_SETTINGS.profiles.label} />
            <div id={`setting-${BROWSER_SETTINGS.profiles.id}`} class="setting-row">
                <div class="setting-devices-description">{BROWSER_SETTINGS.profiles.description}</div>
                <div class="browser-profile-list">
                    <div class="browser-profile-row">
                        <span class="browser-profile-badge browser-profile-badge-personal" aria-hidden="true">
                            <i class="fa-solid fa-user" />
                        </span>
                        <span class="browser-profile-fixed">Personal</span>
                    </div>
                    <For each={browserProfiles()}>{(p) => <ProfileRow profile={p} />}</For>
                </div>
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
                <Show when={error()}>
                    <div class="settings-config-error" role="alert">
                        {error()}
                    </div>
                </Show>
            </div>
            <SettingRow
                id={BROWSER_SETTINGS.pauseAll.id}
                label={BROWSER_SETTINGS.pauseAll.label}
                description={BROWSER_SETTINGS.pauseAll.description}
                control={
                    <ToggleControl
                        checked={browserPauseAll().paused}
                        onChange={(v) => void setAgentBrowsingPaused(v).catch((e) => console.warn("[browser-pause-all] couldn't change it", e))}
                    />
                }
            />
            <SectionHeader label={BROWSER_SETTINGS.agentSites.label} />
            <AgentSiteGrants />
        </div>
    );
}

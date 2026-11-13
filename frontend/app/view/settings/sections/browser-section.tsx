// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Settings → Browser: the named browser profiles — rename, recolour, let
// agents use one, delete, add
// (docs/specs/SPEC_BROWSER_PANE_PROFILES_MENU_2026_10_09.md §3, §5).

import { createSignal, For, onMount, Show, type JSX } from "solid-js";
import { t } from "@/app/i18n/i18n";
import { PaletteSwatches } from "@/app/components/palette-swatches";
import { Button, CloseButton, Switch, TextInput } from "@/app/element/ui";
import { normalizeKeepAwakeSite } from "@/app/view/browser/hibernation";
import type { BrowserProfile } from "@/types/rpc/BrowserProfile";
import {
    agentProfiles,
    createBrowserProfile,
    deleteBrowserProfile,
    profileSignins,
    followBrowserProfiles,
    updateBrowserProfile,
    userProfiles,
} from "@/app/view/browser/browser-profiles";
import { ProfileBadge } from "@/app/view/browser/browser-profile-menu";
import { settingEntry, type SettingsIndexEntry } from "../settings-model";
import { settingsAtom } from "@/app/store/global";
import { NumberControl, SectionHeader, set, SettingRow, ToggleControl } from "../settings-controls";
import { AGENT_SITE_GRANTS_SETTING, AgentSiteGrants } from "./agent-site-grants";
import { browserPauseAll, setAgentBrowsingPaused, startBrowserPauseAll } from "@/app/store/browser-pause-all";

export const BROWSER_SETTINGS = {
    profiles: settingEntry({
        id: "browser.profiles",
        labelKey: "settings.browser.profiles.label",
        descriptionKey: "settings.browser.profiles.description",
        section: "browser",
        keywords: ["profile", "profiles", "account", "accounts", "sign in", "cookies", "incognito", "identity", "work", "agent", "agents"],
    }),
    searchSuggestions: settingEntry({
        id: "browser.search_suggestions",
        labelKey: "settings.browser.searchSuggestions.label",
        descriptionKey: "settings.browser.searchSuggestions.description",
        section: "browser",
        keywords: ["autocomplete", "suggestions", "address bar", "omnibox", "google", "search", "privacy", "browser:searchsuggestions"],
    }),
    hibernate: settingEntry({
        id: "browser.hibernate",
        labelKey: "settings.browser.hibernate.label",
        descriptionKey: "settings.browser.hibernate.description",
        section: "browser",
        keywords: ["hibernate", "sleep", "sleeping", "memory", "discard", "tabs", "browser:hibernate"],
    }),
    hibernateAfter: settingEntry({
        id: "browser.hibernate_after",
        labelKey: "settings.browser.hibernateAfter.label",
        descriptionKey: "settings.browser.hibernateAfter.description",
        section: "browser",
        keywords: ["hibernate", "sleep", "minutes", "delay", "browser:hibernateafter"],
    }),
    keepAwakeSites: settingEntry({
        id: "browser.keep_awake_sites",
        labelKey: "settings.browser.keepAwakeSites.label",
        descriptionKey: "settings.browser.keepAwakeSites.description",
        section: "browser",
        keywords: ["hibernate", "sleep", "keep awake", "never", "sites", "exceptions", "browser:keepawakesites"],
    }),
    pauseAll: settingEntry({
        id: "browser.pause_all",
        labelKey: "settings.browser.pauseAll.label",
        descriptionKey: "settings.browser.pauseAll.description",
        section: "browser",
        keywords: ["pause", "stop", "agent", "agents", "browsing", "emergency", "freeze", "resume", "auto-approve"],
    }),
    agentSites: AGENT_SITE_GRANTS_SETTING,
} satisfies Record<string, SettingsIndexEntry>;

/** The keep-awake sites list: each site with a remove button, and a field to add one. */
function KeepAwakeSites(props: { sites: string[] }): JSX.Element {
    const [draft, setDraft] = createSignal("");
    const [error, setError] = createSignal<string | null>(null);
    const add = () => {
        const site = normalizeKeepAwakeSite(draft());
        if (!site) {
            setError(t("settings.browser.keepAwake.notASite", { site: draft().trim() }));
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
                <ul class="browser-keepawake-list" aria-label={t("settings.browser.keepAwakeSites.label")}>
                    <For each={props.sites}>
                        {(site) => (
                            <li class="browser-keepawake-site">
                                <span>{site}</span>
                                <CloseButton size="sm" label={t("settings.browser.keepAwake.letSleep", { site })} onClick={() => remove(site)} />
                            </li>
                        )}
                    </For>
                </ul>
            </Show>
            <div class="browser-profile-add">
                <TextInput
                    detached
                    aria-label={t("settings.browser.keepAwake.siteInput")}
                    placeholder={"example.com"}
                    value={draft()}
                    onInput={(e) => setDraft(e.currentTarget.value)}
                    onKeyDown={(e) => {
                        if (e.key === "Enter") add();
                    }}
                />
                <Button icon="plus" disabled={!draft().trim()} onClick={add}>
                    {t("settings.browser.keepAwake.add")}
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

/**
 * The Delete button of a profile row. It asks once more on the button itself: delete closes the profile's tabs
 * and signs it out. Its saved passwords may exist nowhere else, so they move to Personal unless the person
 * deletes them too (Vault spec §9.4).
 */
function DeleteProfile(props: { profile: BrowserProfile; run: (f: () => Promise<void>) => Promise<void> }): JSX.Element {
    const [confirming, setConfirming] = createSignal(false);
    let confirmTimer: ReturnType<typeof setTimeout> | undefined;
    const saved = () => profileSignins(props.profile.id);
    const remove = (deleteSignins = false) => {
        if (!confirming()) {
            setConfirming(true);
            confirmTimer = setTimeout(() => setConfirming(false), saved() > 0 ? 10000 : 5000);
            return;
        }
        clearTimeout(confirmTimer);
        setConfirming(false);
        void props.run(() => deleteBrowserProfile(props.profile.id, deleteSignins));
    };
    const signinsWord = () => t("settings.browser.profile.signins", { count: saved() });
    const deleteLabel = () =>
        confirming() ? t("settings.browser.profile.deleteConfirm", { name: props.profile.name }) : t("settings.browser.profile.delete");
    return (
        <Show
            when={confirming() && saved() > 0}
            fallback={
                <Button tone={confirming() ? "danger" : "quiet"} icon="trash" onClick={() => remove()}>
                    {deleteLabel()}
                </Button>
            }
        >
            <div class="browser-profile-delete" role="group" aria-label={t("settings.browser.profile.deleteGroup", { name: props.profile.name, signins: signinsWord() })}>
                <span>{t("settings.browser.profile.deleteAsk", { name: props.profile.name, signins: signinsWord() })}</span>
                <Button tone="danger" onClick={() => remove(false)}>
                    {t("settings.browser.profile.moveToPersonal")}
                </Button>
                <Button tone="quiet" onClick={() => remove(true)}>
                    {t("settings.browser.profile.deleteThemToo")}
                </Button>
            </div>
        </Show>
    );
}

function ProfileRow(props: { profile: BrowserProfile }): JSX.Element {
    const [name, setName] = createSignal(props.profile.name);
    const [error, setError] = createSignal<string | null>(null);

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
    const agentsUseLabel = () =>
        props.profile.owner_agent ? t("settings.browser.profile.otherAgentsMayUse") : t("settings.browser.profile.agentsMayUse");

    return (
        <div class="browser-profile-row">
            <ProfileBadge name={props.profile.name} color={props.profile.color} />
            <Show
                when={!props.profile.owner_agent}
                fallback={
                    <span class="browser-profile-fixed">
                        {props.profile.name} <span class="browser-profile-agent-tag">{t("settings.browser.profile.agentTag")}</span>
                    </span>
                }
            >
                <TextInput
                    detached
                    aria-label={t("settings.browser.profile.nameOf", { name: props.profile.name })}
                    value={name()}
                    maxLength={40}
                    onInput={(e) => setName(e.currentTarget.value)}
                    onBlur={rename}
                    onKeyDown={(e) => {
                        if (e.key === "Enter") (e.currentTarget as HTMLInputElement).blur();
                        if (e.key === "Escape") setName(props.profile.name);
                    }}
                />
                <PaletteSwatches
                    label={t("settings.browser.profile.colourOf", { name: props.profile.name })}
                    value={props.profile.color}
                    onSelect={(slot) => slot && void run(() => updateBrowserProfile(props.profile.id, { color: slot.hex }))}
                />
            </Show>
            <label class="browser-profile-agents">
                <Switch
                    checked={props.profile.agents_allowed}
                    ariaLabel={t("settings.browser.profile.agentsMayUseName", { name: props.profile.name })}
                    onChange={(on) => void run(() => updateBrowserProfile(props.profile.id, { agents_allowed: on }))}
                />
                <span>{agentsUseLabel()}</span>
            </label>
            <DeleteProfile profile={props.profile} run={run} />
            <Show when={error()}>
                <div class="settings-config-error" role="alert">
                    {error()}
                </div>
            </Show>
        </div>
    );
}

/** The user's profiles, then each agent's own, which AgentMux makes the first time that agent opens a browser. */
function ProfileRows(): JSX.Element {
    return (
        <>
            <For each={userProfiles()}>{(p) => <ProfileRow profile={p} />}</For>
            <AgentProfiles />
        </>
    );
}

function AgentProfiles(): JSX.Element {
    return (
        <Show when={agentProfiles().length > 0}>
            <div class="setting-devices-description">
                {t("settings.browser.profile.agentsNote")}
            </div>
            <div class="browser-profile-list" aria-label={t("settings.browser.profile.agentsListLabel")}>
                <For each={agentProfiles()}>{(p) => <ProfileRow profile={p} />}</For>
            </div>
        </Show>
    );
}

export function BrowserSection(): JSX.Element {
    const [newName, setNewName] = createSignal("");
    const [error, setError] = createSignal<string | null>(null);
    followBrowserProfiles();
    onMount(() => {
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
                        <span class="browser-profile-fixed">{t("settings.browser.profile.personal")}</span>
                    </div>
                    <ProfileRows />
                </div>
                <div class="browser-profile-add">
                    <TextInput
                        detached
                        aria-label={t("settings.browser.profile.newName")}
                        placeholder={t("settings.browser.profile.newName")}
                        value={newName()}
                        maxLength={40}
                        onInput={(e) => setNewName(e.currentTarget.value)}
                        onKeyDown={(e) => {
                            if (e.key === "Enter") void add();
                        }}
                    />
                    <Button icon="plus" disabled={!newName().trim()} onClick={() => void add()}>
                        {t("settings.browser.profile.add")}
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

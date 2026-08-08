// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Per-block / per-connection SolidJS signal caches and derived
// settings/meta memo helpers. Extracted from global.ts so that
// layout/lib/layoutModel.ts can import getSettingsKeyAtom without
// touching the global god-module.

import { createMemo, createRoot } from "solid-js";
import { deepCompareReturnPrev, getPrefixedSettings } from "@/util/util";
import { fullConfigAtom, settingsAtom } from "./config-signals";
import * as WOS from "./wos";

// ---------------------------------------------------------------------------
// Block atom caches (used by per-block derived memos)
// ---------------------------------------------------------------------------

const blockAtomCache = new Map<string, Map<string, () => any>>();
const blockAtomDisposers = new Map<string, (() => void)[]>();

function getSingleBlockAtomCache(blockId: string): Map<string, () => any> {
    let bc = blockAtomCache.get(blockId);
    if (bc == null) {
        bc = new Map();
        blockAtomCache.set(blockId, bc);
    }
    return bc;
}

function addBlockAtomDisposer(blockId: string, dispose: () => void) {
    let disposers = blockAtomDisposers.get(blockId);
    if (disposers == null) {
        disposers = [];
        blockAtomDisposers.set(blockId, disposers);
    }
    disposers.push(dispose);
}

export function cleanupBlockAtomCache(blockId: string) {
    blockAtomCache.delete(blockId);
    const disposers = blockAtomDisposers.get(blockId);
    if (disposers) {
        for (const dispose of disposers) {
            try { dispose(); } catch (_) {}
        }
        blockAtomDisposers.delete(blockId);
    }
}

function getSingleConnAtomCache(connName: string): Map<string, () => any> {
    return getSingleBlockAtomCache(connName);
}

export function getBlockMetaKeyAtom<T extends keyof MetaType>(blockId: string, key: T): () => MetaType[T] {
    const bc = getSingleBlockAtomCache(blockId);
    const name = "#meta-" + key;
    let memo = bc.get(name);
    if (memo == null) {
        memo = createRoot((dispose) => {
            addBlockAtomDisposer(blockId, dispose);
            return createMemo(() => {
                const blockAccessor = WOS.getWaveObjectAtom(WOS.makeORef("block", blockId));
                const blockData = blockAccessor();
                return blockData?.meta?.[key];
            });
        });
        bc.set(name, memo);
    }
    return memo as () => MetaType[T];
}

// ---------------------------------------------------------------------------
// Connection config
// ---------------------------------------------------------------------------

function getConnConfigKeyAtom<T extends keyof ConnKeywords>(connName: string, key: T): () => ConnKeywords[T] {
    const cc = getSingleConnAtomCache(connName);
    const name = "#conn-" + key;
    let memo = cc.get(name);
    if (memo == null) {
        memo = createRoot((dispose) => {
            addBlockAtomDisposer(connName, dispose);
            return createMemo(() => fullConfigAtom()?.connections?.[connName]?.[key]);
        });
        cc.set(name, memo);
    }
    return memo as () => ConnKeywords[T];
}

// ---------------------------------------------------------------------------
// Settings atoms
// ---------------------------------------------------------------------------

const settingsAtomCache = new Map<string, () => any>();

export function getSettingsKeyAtom<T extends keyof SettingsType>(key: T): () => SettingsType[T] {
    let memo = settingsAtomCache.get(key) as () => SettingsType[T];
    if (memo == null) {
        memo = createRoot(() => createMemo(() => {
            const settings = settingsAtom();
            if (settings == null) return null;
            return settings[key];
        }));
        settingsAtomCache.set(key, memo);
    }
    return memo;
}

export function getOverrideConfigAtom<T extends keyof SettingsType>(blockId: string, key: T): () => SettingsType[T] {
    const bc = getSingleBlockAtomCache(blockId);
    const name = "#settingsoverride-" + key;
    let memo = bc.get(name);
    if (memo == null) {
        memo = createRoot((dispose) => {
            addBlockAtomDisposer(blockId, dispose);
            return createMemo(() => {
                const metaKeyMemo = getBlockMetaKeyAtom(blockId, key as any);
                const metaKeyVal = metaKeyMemo();
                if (metaKeyVal != null) return metaKeyVal as SettingsType[T];

                const connNameMemo = getBlockMetaKeyAtom(blockId, "connection");
                const connName = connNameMemo();
                const connConfigKeyMemo = getConnConfigKeyAtom(connName, key as any);
                const connConfigKeyVal = connConfigKeyMemo();
                if (connConfigKeyVal != null) return connConfigKeyVal as SettingsType[T];

                const settingsKeyMemo = getSettingsKeyAtom(key);
                const settingsVal = settingsKeyMemo();
                if (settingsVal != null) return settingsVal;

                return null;
            });
        });
        bc.set(name, memo);
    }
    return memo as () => SettingsType[T];
}

const settingsPrefixCache = new Map<string, () => SettingsType>();

export function getSettingsPrefixAtom(prefix: string): () => SettingsType {
    let memo = settingsPrefixCache.get(prefix + ":");
    if (memo == null) {
        const cacheKey = {};
        memo = createRoot(() => createMemo(() => {
            const settings = settingsAtom();
            const newValue = getPrefixedSettings(settings, prefix);
            return deepCompareReturnPrev(cacheKey, newValue);
        }));
        settingsPrefixCache.set(prefix + ":", memo);
    }
    return memo;
}

// ---------------------------------------------------------------------------
// Block atom cache (used by block components to store per-block memos)
// ---------------------------------------------------------------------------

export function useBlockAtom<T>(blockId: string, name: string, makeFn: () => () => T): () => T {
    const bc = getSingleBlockAtomCache(blockId);
    let memo = bc.get(name);
    if (memo == null) {
        memo = createRoot(makeFn);
        bc.set(name, memo);
        console.log("New BlockAtom", blockId, name);
    }
    return memo as () => T;
}

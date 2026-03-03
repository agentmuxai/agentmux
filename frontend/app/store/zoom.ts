// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { getApi } from "@/app/store/global";
import { atom } from "jotai";

// Zoom constants
export const MIN_ZOOM = 0.25;
export const MAX_ZOOM = 3.0;
export const DEFAULT_ZOOM = 1.0;
export const KEYBOARD_STEP = 0.1; // 10%
export const WHEEL_STEP = 0.05; // 5%

// Current zoom level atom
export const zoomFactorAtom = atom<number>(DEFAULT_ZOOM);

// Zoom indicator visibility (auto-hide after 1.5s)
export const zoomIndicatorVisibleAtom = atom<boolean>(false);
let zoomIndicatorTimeout: NodeJS.Timeout | null = null;

/**
 * Clamp zoom factor to valid range
 */
function clampZoom(factor: number): number {
    return Math.min(Math.max(factor, MIN_ZOOM), MAX_ZOOM);
}

/**
 * Round to nearest 5% for clean display
 */
function roundZoom(factor: number): number {
    return Math.round(factor * 20) / 20; // Round to 0.05 increments
}

/**
 * Set zoom factor and update UI
 */
export function setZoom(factor: number, store: any): void {
    const clampedZoom = clampZoom(roundZoom(factor));

    // Update atom
    store.set(zoomFactorAtom, clampedZoom);

    // Apply to Tauri window
    const api = getApi();
    if (api && typeof api.setZoomFactor === "function") {
        api.setZoomFactor(clampedZoom);
    }

    // Persist to settings
    persistZoom(clampedZoom);

    // Show indicator
    showZoomIndicator(store);
}

/**
 * Increase zoom by step
 */
export function zoomIn(store: any, step: number = KEYBOARD_STEP): void {
    const current = store.get(zoomFactorAtom);
    setZoom(current + step, store);
}

/**
 * Decrease zoom by step
 */
export function zoomOut(store: any, step: number = KEYBOARD_STEP): void {
    const current = store.get(zoomFactorAtom);
    setZoom(current - step, store);
}

/**
 * Reset zoom to 100%
 */
export function zoomReset(store: any): void {
    setZoom(DEFAULT_ZOOM, store);
}

/**
 * Persist zoom level to user settings
 * Note: Persistence is handled by Rust backend in AppState.zoom_factor
 */
async function persistZoom(factor: number): Promise<void> {
    // Zoom factor is automatically persisted by the Rust backend
    // when set_zoom_factor command is called
}

/**
 * Load zoom level from settings on startup
 * Note: Zoom factor is loaded from Rust AppState via getZoomFactor
 */
export async function loadZoom(store: any): Promise<void> {
    // Get current zoom from Tauri (which loads from AppState)
    const api = getApi();
    const currentZoom = (api && typeof api.getZoomFactor === "function" ? api.getZoomFactor() : null) ?? DEFAULT_ZOOM;
    store.set(zoomFactorAtom, currentZoom);
}

/**
 * Show zoom indicator with auto-hide
 */
function showZoomIndicator(store: any): void {
    // Clear existing timeout
    if (zoomIndicatorTimeout) {
        clearTimeout(zoomIndicatorTimeout);
    }

    // Show indicator
    store.set(zoomIndicatorVisibleAtom, true);

    // Hide after 1.5 seconds
    zoomIndicatorTimeout = setTimeout(() => {
        store.set(zoomIndicatorVisibleAtom, false);
        zoomIndicatorTimeout = null;
    }, 1500);
}

/**
 * Get zoom as percentage string
 */
export function getZoomPercentage(store: any): string {
    const zoom = store.get(zoomFactorAtom);
    return `${Math.round(zoom * 100)}%`;
}

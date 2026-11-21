// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * True when this window's page carries the window opacity itself, so menus, submenus and popovers stay solid at any
 * opacity (#48092). The host starts a process that way only when the transparency setting is on or a window was below
 * 100% the last time the app ran (native Wayland always is); otherwise it keeps sub-pixel text smoothing, fades the
 * window with the operating system for that session, and menus fade with it until the next start. The host says which
 * in a page global, set with the IPC credentials in every window.
 */
export function pageCarriesWindowFade(host: { __AGENTMUX_PAGE_FADE__?: boolean } = window): boolean {
    return host.__AGENTMUX_PAGE_FADE__ === true;
}

/** The note the Opacity menu shows when menus fade with the window, until the next start. */
export const OPACITY_MENUS_NOTE = "Menus turn solid after a restart";

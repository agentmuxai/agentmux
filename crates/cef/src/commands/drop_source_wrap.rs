// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Windows: an arrow instead of the circle-slash while a tab or pane is dragged out of every AgentMux window.
// libcef reaches `DoDragDrop` through its delay-load table; one entry is swapped for a function that hands
// it a drop source wrapping Chromium's, which answers "nothing accepts" with an arrow during a tear-off
// hook session. Any failure leaves the table alone.
// docs/reports/REPORT_DRAG_CURSOR_CIRCLE_SLASH_REASSESSED_2026_11_05.md

use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};

use windows_sys::Win32::Foundation::{BOOL, HMODULE};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Memory::{VirtualProtect, PAGE_READWRITE};
use windows_sys::Win32::System::Ole::DoDragDrop;
use windows_sys::Win32::UI::WindowsAndMessaging::{LoadCursorW, SetCursor, IDC_ARROW};

type HResult = i32;
const S_OK: HResult = 0;
const E_NOINTERFACE: HResult = 0x8000_4002_u32 as i32;
/// The three drop effects a target can accept (`DROPEFFECT_COPY | MOVE | LINK`); `SCROLL` is a flag on top.
const EFFECT_MASK: u32 = 0x7;

/// Whether to draw an arrow for the effect the window under the pointer returned.
fn wants_arrow(effect: u32, tear_off_session: bool) -> bool {
    tear_off_session && effect & EFFECT_MASK == 0
}

/// A drop source's vtable (`IDropSource`, after `IUnknown`).
#[repr(C)]
struct DropSourceVtbl {
    query_interface: unsafe extern "system" fn(*mut c_void, *const [u8; 16], *mut *mut c_void) -> HResult,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    query_continue_drag: unsafe extern "system" fn(*mut c_void, BOOL, u32) -> HResult,
    give_feedback: unsafe extern "system" fn(*mut c_void, u32) -> HResult,
}

/// Any COM object that starts with a vtable pointer: Chromium's own drop source.
#[repr(C)]
struct RawDropSource {
    vtbl: *const DropSourceVtbl,
}

/// Our drop source: Chromium's, with the cursor answered for the one case it leaves to Windows.
#[repr(C)]
struct Wrapper {
    vtbl: *const DropSourceVtbl,
    refs: AtomicU32,
    inner: *mut RawDropSource,
}

static WRAPPER_VTBL: DropSourceVtbl = DropSourceVtbl {
    query_interface: wrapper_query_interface,
    add_ref: wrapper_add_ref,
    release: wrapper_release,
    query_continue_drag: wrapper_query_continue_drag,
    give_feedback: wrapper_give_feedback,
};

/// `IID_IUnknown` and `IID_IDropSource`, as the bytes `QueryInterface` is given.
const IID_IUNKNOWN: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46];
const IID_IDROPSOURCE: [u8; 16] = [0x21, 1, 0, 0, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46];

unsafe extern "system" fn wrapper_query_interface(
    this: *mut c_void,
    iid: *const [u8; 16],
    out: *mut *mut c_void,
) -> HResult {
    if iid.is_null() || out.is_null() {
        return E_NOINTERFACE;
    }
    if *iid == IID_IUNKNOWN || *iid == IID_IDROPSOURCE {
        wrapper_add_ref(this);
        *out = this;
        return S_OK;
    }
    *out = std::ptr::null_mut();
    E_NOINTERFACE
}

unsafe extern "system" fn wrapper_add_ref(this: *mut c_void) -> u32 {
    (*(this as *mut Wrapper)).refs.fetch_add(1, Ordering::AcqRel) + 1
}

unsafe extern "system" fn wrapper_release(this: *mut c_void) -> u32 {
    let left = (*(this as *mut Wrapper)).refs.fetch_sub(1, Ordering::AcqRel) - 1;
    if left == 0 {
        let wrapper = Box::from_raw(this as *mut Wrapper);
        ((*(*wrapper.inner).vtbl).release)(wrapper.inner as *mut c_void);
    }
    left
}

unsafe extern "system" fn wrapper_query_continue_drag(this: *mut c_void, escape: BOOL, keys: u32) -> HResult {
    let inner = (*(this as *mut Wrapper)).inner;
    ((*(*inner).vtbl).query_continue_drag)(inner as *mut c_void, escape, keys)
}

unsafe extern "system" fn wrapper_give_feedback(this: *mut c_void, effect: u32) -> HResult {
    let inner = (*(this as *mut Wrapper)).inner;
    let answer = ((*(*inner).vtbl).give_feedback)(inner as *mut c_void, effect);
    if wants_arrow(effect, super::tear_off_hook::hook_session_active()) {
        SetCursor(LoadCursorW(std::ptr::null_mut(), IDC_ARROW));
        return S_OK;
    }
    answer
}

/// What libcef calls instead of `DoDragDrop`: the same drag, with our drop source around Chromium's.
unsafe extern "system" fn do_drag_drop_wrapped(
    data: *mut c_void,
    source: *mut c_void,
    ok_effects: u32,
    effect: *mut u32,
) -> HResult {
    if source.is_null() {
        return DoDragDrop(data, source, ok_effects, effect);
    }
    let inner = source as *mut RawDropSource;
    ((*(*inner).vtbl).add_ref)(source);
    let wrapper = Box::into_raw(Box::new(Wrapper { vtbl: &WRAPPER_VTBL, refs: AtomicU32::new(1), inner }));
    let result = DoDragDrop(data, wrapper as *mut c_void, ok_effects, effect);
    wrapper_release(wrapper as *mut c_void);
    result
}

/// The address of libcef's delay-load slot for `ole32.dll!DoDragDrop`, or why there is none.
unsafe fn find_slot(module: HMODULE) -> Result<*mut usize, &'static str> {
    let base = module as *const u8;
    let u32_at = |off: usize| (base.add(off) as *const u32).read_unaligned();
    if (base as *const u16).read_unaligned() != 0x5A4D {
        return Err("no DOS header");
    }
    let nt = u32_at(0x3c) as usize;
    if u32_at(nt) != 0x0000_4550 {
        return Err("no NT header");
    }
    // x64 optional header: the data directories start 112 bytes in, after the 24-byte file header.
    let optional = nt + 24;
    if (base.add(optional) as *const u16).read_unaligned() != 0x20b {
        return Err("not a 64-bit image");
    }
    let delay_dir = optional + 112 + 13 * 8;
    let mut descriptor = u32_at(delay_dir) as usize;
    if descriptor == 0 {
        return Err("no delay-load table");
    }
    while u32_at(descriptor + 4) != 0 {
        let rva_based = u32_at(descriptor) & 1 == 1;
        let name = std::ffi::CStr::from_ptr(base.add(u32_at(descriptor + 4) as usize) as *const _);
        if rva_based && name.to_bytes().eq_ignore_ascii_case(b"ole32.dll") {
            let iat = u32_at(descriptor + 12) as usize;
            let names = u32_at(descriptor + 16) as usize;
            for i in 0.. {
                let entry = (base.add(names + i * 8) as *const u64).read_unaligned();
                if entry == 0 {
                    break;
                }
                if entry >> 63 == 1 {
                    continue; // by ordinal
                }
                let symbol = std::ffi::CStr::from_ptr(base.add(entry as usize + 2) as *const _);
                if symbol.to_bytes() == b"DoDragDrop" {
                    return Ok(base.add(iat + i * 8) as *mut usize);
                }
            }
        }
        descriptor += 32;
    }
    Err("DoDragDrop is not in libcef's delay-load table")
}

/// Swap libcef's `DoDragDrop` for the wrapped one. `AGENTMUX_DND_CURSOR=0` leaves it alone. Never fails the
/// app: on any problem the slash stays and the reason is logged.
pub fn install() {
    if std::env::var("AGENTMUX_DND_CURSOR").is_ok_and(|v| v == "0") {
        tracing::info!("[drag-cursor] disabled by AGENTMUX_DND_CURSOR=0");
        return;
    }
    unsafe {
        let name: Vec<u16> = "libcef.dll\0".encode_utf16().collect();
        let module = GetModuleHandleW(name.as_ptr());
        if module.is_null() {
            tracing::warn!("[drag-cursor] libcef.dll is not loaded; the drag cursor stays as it is");
            return;
        }
        let slot = match find_slot(module) {
            Ok(slot) => slot,
            Err(why) => {
                tracing::warn!(why, "[drag-cursor] cannot find libcef's DoDragDrop; the drag cursor stays as it is");
                return;
            }
        };
        let mut old = 0;
        if VirtualProtect(slot as *const c_void, 8, PAGE_READWRITE, &mut old) == 0 {
            tracing::warn!("[drag-cursor] cannot make libcef's delay-load table writable");
            return;
        }
        slot.write(do_drag_drop_wrapped as *const () as usize);
        VirtualProtect(slot as *const c_void, 8, old, &mut old);
    }
    tracing::info!("[drag-cursor] libcef's DoDragDrop now goes through the AgentMux drop source");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_accepting_the_drag_gets_an_arrow_during_a_tear_off() {
        assert!(wants_arrow(0, true));
        assert!(wants_arrow(0x8000_0000, true)); // the scroll flag alone is still "nothing accepts"
    }

    #[test]
    fn an_accepting_target_keeps_its_own_cursor() {
        assert!(!wants_arrow(1, true));
        assert!(!wants_arrow(2, true));
        assert!(!wants_arrow(0x8000_0001, true));
    }

    #[test]
    fn other_drags_keep_the_default_cursor() {
        assert!(!wants_arrow(0, false));
    }

    #[test]
    fn the_interface_ids_are_unknown_and_drop_source() {
        assert_eq!(IID_IUNKNOWN[8], 0xC0);
        assert_eq!(IID_IDROPSOURCE[0], 0x21);
        assert_eq!(IID_IDROPSOURCE[1], 0x01);
    }
}

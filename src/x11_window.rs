// SPDX-License-Identifier: GPL-3.0-or-later
//! Set `_NET_WM_ICON` and `WM_CLASS` on X11 windows.
//!
//! `iced_winit` currently wraps window icons in a private `RawImage` type that
//! `winit-x11` fails to downcast to `RgbaIcon` (leaving `_NET_WM_ICON` empty),
//! and overwrites `WindowAttributesX11` with `WindowAttributesWayland` when
//! both backends are enabled (leaving `WM_CLASS` as the executable name).
//! Setting both properties directly via the already-loaded `libX11.so.6` fixes
//! X11 taskbars, panels and Alt-Tab switchers while remaining a no-op on
//! Wayland.

use cosmic::iced::window::raw_window_handle::{
    HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
};
use std::ffi::{CStr, c_char, c_int, c_uchar, c_ulong, c_void};
use std::ptr;

const RTLD_LAZY: c_int = 1;
const RTLD_NOLOAD: c_int = 4;
const XA_CARDINAL: c_ulong = 6;
const XA_STRING: c_ulong = 31;
const XA_WM_CLASS: c_ulong = 67;
const PROP_MODE_REPLACE: c_int = 0;

unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
}

type XOpenDisplayFn = unsafe extern "C" fn(*const c_char) -> *mut c_void;
type XCloseDisplayFn = unsafe extern "C" fn(*mut c_void) -> c_int;
type XInternAtomFn = unsafe extern "C" fn(*mut c_void, *const c_char, c_int) -> c_ulong;
type XChangePropertyFn = unsafe extern "C" fn(
    *mut c_void,
    c_ulong,
    c_ulong,
    c_ulong,
    c_int,
    c_int,
    *const c_uchar,
    c_int,
) -> c_int;
type XFlushFn = unsafe extern "C" fn(*mut c_void) -> c_int;

/// Encode one or more square sizes of the bundled app icon into the EWMH
/// `_NET_WM_ICON` format (`[width, height, 0xAARRGGBB, ...]`).
///
/// Xlib `XChangeProperty` with `format = 32` expects an array of `c_ulong`,
/// even on 64-bit platforms where each element carries one 32-bit cardinal.
pub fn ewmh_icon_cardinals(sizes: &[u32]) -> Option<Vec<c_ulong>> {
    const PNG: &[u8] = include_bytes!("../resources/dev.nilltadios.GearLeverCosmic.png");
    let img = image::load_from_memory_with_format(PNG, image::ImageFormat::Png)
        .ok()?
        .to_rgba8();
    let mut out = Vec::new();
    for &side in sizes {
        let resized =
            image::imageops::resize(&img, side, side, image::imageops::FilterType::Triangle);
        let (w, h) = resized.dimensions();
        out.reserve(2 + (w as usize) * (h as usize));
        out.push(c_ulong::from(w));
        out.push(c_ulong::from(h));
        for px in resized.pixels() {
            let [r, g, b, a] = px.0;
            let argb =
                (u32::from(a) << 24) | (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b);
            out.push(c_ulong::from(argb));
        }
    }
    Some(out)
}

unsafe fn load_sym<T: Copy>(lib: *mut c_void, name: &CStr) -> Option<T> {
    let ptr = unsafe { dlsym(lib, name.as_ptr()) };
    if ptr.is_null() {
        None
    } else {
        Some(unsafe { std::mem::transmute_copy(&ptr) })
    }
}

/// Apply `_NET_WM_ICON` and `WM_CLASS` if `window` is backed by X11.
/// Does nothing on Wayland or any non-X11 backend.
pub fn apply_icon_and_class<W>(window: &W, app_id: &str)
where
    W: HasWindowHandle + HasDisplayHandle + ?Sized,
{
    let xwindow: c_ulong = match window.window_handle().map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Xlib(h)) => h.window,
        Ok(RawWindowHandle::Xcb(h)) => c_ulong::from(h.window.get()),
        _ => return,
    };
    let borrowed_display: Option<*mut c_void> = match window.display_handle().map(|h| h.as_raw()) {
        Ok(RawDisplayHandle::Xlib(h)) => h.display.map(std::ptr::NonNull::as_ptr),
        _ => None,
    };

    // SAFETY: We only look up symbols in the already-loaded libX11.so.6 (via
    // RTLD_NOLOAD) and pass valid X11 window/display handles on the winit
    // event-loop thread.
    unsafe {
        let lib = dlopen(c"libX11.so.6".as_ptr(), RTLD_LAZY | RTLD_NOLOAD);
        if lib.is_null() {
            return;
        }
        let result = (|| -> Option<()> {
            let x_intern_atom: XInternAtomFn = load_sym(lib, c"XInternAtom")?;
            let x_change_property: XChangePropertyFn = load_sym(lib, c"XChangeProperty")?;
            let x_flush: XFlushFn = load_sym(lib, c"XFlush")?;

            let (display, owned) = match borrowed_display {
                Some(d) => (d, false),
                None => {
                    let x_open: XOpenDisplayFn = load_sym(lib, c"XOpenDisplay")?;
                    let d = x_open(ptr::null());
                    if d.is_null() {
                        return None;
                    }
                    (d, true)
                }
            };

            if let Some(cardinals) = ewmh_icon_cardinals(&[128, 48]) {
                let net_wm_icon = x_intern_atom(display, c"_NET_WM_ICON".as_ptr(), 0);
                if net_wm_icon != 0 {
                    x_change_property(
                        display,
                        xwindow,
                        net_wm_icon,
                        XA_CARDINAL,
                        32,
                        PROP_MODE_REPLACE,
                        cardinals.as_ptr().cast::<c_uchar>(),
                        cardinals.len() as c_int,
                    );
                }
            }

            // ICCCM WM_CLASS: two NUL-terminated strings: instance\0class\0.
            let mut wm_class = Vec::with_capacity(app_id.len() * 2 + 2);
            wm_class.extend_from_slice(app_id.as_bytes());
            wm_class.push(0);
            wm_class.extend_from_slice(app_id.as_bytes());
            wm_class.push(0);
            x_change_property(
                display,
                xwindow,
                XA_WM_CLASS,
                XA_STRING,
                8,
                PROP_MODE_REPLACE,
                wm_class.as_ptr(),
                wm_class.len() as c_int,
            );

            x_flush(display);
            if owned && let Some(x_close) = load_sym::<XCloseDisplayFn>(lib, c"XCloseDisplay") {
                x_close(display);
            }
            Some(())
        })();
        let _ = result;
        dlclose(lib);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ewmh_icon_encodes_requested_sizes_with_opaque_pixels() {
        let data = ewmh_icon_cardinals(&[128, 48]).expect("bundled PNG decodes");
        assert_eq!(data.len(), (2 + 128 * 128) + (2 + 48 * 48));
        assert_eq!(data[0], 128);
        assert_eq!(data[1], 128);
        assert_eq!(data[2 + 128 * 128], 48);
        assert_eq!(data[2 + 128 * 128 + 1], 48);
        // Center pixel of the logo must be visible (non-zero alpha).
        let center = data[2 + 64 * 128 + 64];
        assert_ne!(center >> 24, 0, "center pixel should not be transparent");
    }
}

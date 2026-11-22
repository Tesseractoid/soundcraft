use clap_sys::ext::gui::clap_window_handle;
use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSApplication, NSBackingStoreType, NSFloatingWindowLevel, NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use std::ffi::c_void;

pub const SUPPORTED: bool = true;

/// An `NSWindow` (titled, closable, floating above the app's windows) whose content view is the
/// plugin's parent `NSView`, as the VST3 host does. Holds one retain of the window; main thread only.
pub struct HostWindow {
    window: *mut NSWindow,
}

// SAFETY: the pointer is only dereferenced on the main thread (every method checks
// `MainThreadMarker`); the struct merely carries the retain between threads' locks.
unsafe impl Send for HostWindow {}

impl HostWindow {
    fn get(&self) -> Option<&NSWindow> {
        MainThreadMarker::new()?;
        // SAFETY: we hold a retain of this window (from `new`) until `close`.
        unsafe { self.window.as_ref() }
    }

    pub fn new(title: &str, w: f64, h: f64) -> Result<HostWindow, String> {
        let mtm = MainThreadMarker::new().ok_or_else(|| "not on the main thread".to_string())?;
        let _app = NSApplication::sharedApplication(mtm);
        let rect = NSRect::new(NSPoint::new(200.0, 200.0), NSSize::new(w, h));
        // Not minimizable: minimized NSWindows become persistent thumbnails in the Dock.
        let style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable;
        // SAFETY: a freshly allocated window on the main thread; valid rect and style.
        let window =
            unsafe { NSWindow::initWithContentRect_styleMask_backing_defer(NSWindow::alloc(mtm), rect, style, NSBackingStoreType::Buffered, false) };
        // SAFETY: we keep our own retain and release it in `close`, so AppKit must not
        // release the window when the user closes it.
        unsafe { window.setReleasedWhenClosed(false) };
        window.setTitle(&NSString::from_str(title));
        window.setLevel(NSFloatingWindowLevel);
        window.setHidesOnDeactivate(true);
        window.setExcludedFromWindowsMenu(true);
        window.setCollectionBehavior(NSWindowCollectionBehavior::Transient | NSWindowCollectionBehavior::IgnoresCycle);
        window.center();
        Ok(HostWindow { window: Retained::into_raw(window) })
    }

    /// The raw window pointer (for resizes).
    pub fn raw(&self) -> *mut c_void {
        self.window.cast()
    }

    /// The content view (`NSView*`) the plugin's GUI attaches to; null when unavailable.
    pub fn parent(&self) -> *mut c_void {
        self.get().and_then(|w| w.contentView()).map_or(std::ptr::null_mut(), |v| Retained::as_ptr(&v).cast_mut().cast())
    }

    pub fn handle(&self) -> clap_window_handle {
        clap_window_handle { cocoa: self.parent() }
    }

    pub fn raise(&self) {
        if let Some(w) = self.get() {
            w.makeKeyAndOrderFront(None);
        }
    }

    pub fn is_visible(&self) -> bool {
        self.get().is_some_and(|w| w.isVisible())
    }

    pub fn close(self) {
        if let Some(w) = self.get() {
            w.orderOut(None);
            w.close();
        }
        if MainThreadMarker::new().is_some() {
            // SAFETY: balances the retain taken in `new`; nothing uses the pointer after this.
            drop(unsafe { Retained::from_raw(self.window) });
        }
    }
}

/// Resizes a host window's content (`window` from [`HostWindow::raw`]); main thread only.
pub fn resize(window: *mut c_void, w: f64, h: f64) {
    if MainThreadMarker::new().is_none() {
        return;
    }
    // SAFETY: null or a window the editor link keeps retained while the GUI is attached.
    if let Some(win) = unsafe { window.cast::<NSWindow>().as_ref() } {
        win.setContentSize(NSSize::new(w, h));
    }
}

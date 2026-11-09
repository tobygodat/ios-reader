//! C ABI the Swift shell in `ios/` calls. The contract is `ios/Reader/ReaderCore.h`.
//!
//! Every entry point is null-safe and catches panics, so none unwinds into Swift.
//!
//! Waking: Rune's `Waker` may fire on any thread (a parse finishing). It posts one coalesced
//! block to the main queue, which calls the host's `ReaderWakeFn` so Swift resumes its paused
//! display link. Timer deadlines (`EmbedFrame::wake_after_secs`) ride the same path through
//! `dispatch_after`. `reader_host_free` clears the callback first, so a block still in flight
//! after the view is gone calls nothing.

use std::ffi::{CStr, OsStr, c_char, c_void};
use std::os::unix::ffi::OsStrExt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use reader_app::{Host, HostRequest, State};
use rune::TouchPhase;
use rune::embed::LayerHost;

const REQUEST_NONE: u32 = 0;
const REQUEST_PICK_BOOK: u32 = 1;
const REQUEST_FIND_BOOKS: u32 = 2;
const REQUEST_SIGN_IN: u32 = 3;
const REQUEST_CREATE_ACCOUNT: u32 = 4;
const REQUEST_ADD_FOLDER: u32 = 5;
const REQUEST_FORGET_FOLDERS: u32 = 6;

pub struct ReaderHost {
    view: LayerHost<State>,
    wake: Arc<Wake>,
    insets: [f32; 4],
}

type WakeFn = extern "C" fn(*mut c_void);

#[derive(Clone, Copy)]
struct WakeTarget {
    f: WakeFn,
    ctx: *mut c_void,
}

// The target is only called on the main thread, where Swift registered it.
unsafe impl Send for WakeTarget {}

struct Wake {
    target: Mutex<Option<WakeTarget>>,
    queued: AtomicBool,
}

impl Wake {
    fn ring(self: &Arc<Self>) {
        if self.queued.swap(true, Ordering::AcqRel) {
            return;
        }
        let ctx = Arc::into_raw(self.clone()) as *mut c_void;
        unsafe { dispatch_async_f(&raw const _dispatch_main_q, ctx, fire_queued) };
    }

    fn ring_after(self: &Arc<Self>, secs: f64) {
        let ctx = Arc::into_raw(self.clone()) as *mut c_void;
        let delta = (secs.max(0.0) * 1e9).min(i64::MAX as f64) as i64;
        unsafe { dispatch_after_f(dispatch_time(DISPATCH_TIME_NOW, delta), &raw const _dispatch_main_q, ctx, fire_timer) };
    }

    fn call(&self) {
        let target = *self.target.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = target {
            (t.f)(t.ctx);
        }
    }

    fn clear(&self) {
        *self.target.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

extern "C" fn fire_queued(ctx: *mut c_void) {
    let wake = unsafe { Arc::from_raw(ctx as *const Wake) };
    wake.queued.store(false, Ordering::Release);
    guarded((), || wake.call());
}

extern "C" fn fire_timer(ctx: *mut c_void) {
    let wake = unsafe { Arc::from_raw(ctx as *const Wake) };
    guarded((), || wake.call());
}

#[repr(C)]
struct DispatchQueue {
    _opaque: [u8; 0],
}

const DISPATCH_TIME_NOW: u64 = 0;

unsafe extern "C" {
    static _dispatch_main_q: DispatchQueue;
    fn dispatch_async_f(queue: *const DispatchQueue, ctx: *mut c_void, work: extern "C" fn(*mut c_void));
    fn dispatch_after_f(when: u64, queue: *const DispatchQueue, ctx: *mut c_void, work: extern "C" fn(*mut c_void));
    fn dispatch_time(when: u64, delta: i64) -> u64;
}

fn guarded<T>(fallback: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(fallback)
}

fn with_host<T>(host: *mut ReaderHost, fallback: T, f: impl FnOnce(&mut ReaderHost) -> T) -> T {
    match unsafe { host.as_mut() } {
        Some(host) => guarded(fallback, || f(host)),
        None => fallback,
    }
}

fn path_arg(path: *const c_char) -> Option<PathBuf> {
    if path.is_null() {
        return None;
    }
    let bytes = unsafe { CStr::from_ptr(path) }.to_bytes();
    (!bytes.is_empty()).then(|| PathBuf::from(OsStr::from_bytes(bytes)))
}

/// # Safety
/// `layer` is a `CAMetalLayer *` that outlives the host; `data_dir` is a NUL-terminated path.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reader_host_new(
    layer: *mut c_void,
    width: f32,
    height: f32,
    scale: f32,
    dark: bool,
    data_dir: *const c_char,
    wake: Option<WakeFn>,
    wake_ctx: *mut c_void,
) -> *mut ReaderHost {
    guarded(std::ptr::null_mut(), || {
        let Some(data_dir) = path_arg(data_dir) else { return std::ptr::null_mut() };
        if layer.is_null() {
            return std::ptr::null_mut();
        }
        reader_app::configure(data_dir, Host::Ios);
        let wake = Arc::new(Wake {
            target: Mutex::new(wake.map(|f| WakeTarget { f, ctx: wake_ctx })),
            queued: AtomicBool::new(false),
        });
        let ring = wake.clone();
        let spec = reader_app::spec();
        let state = (spec.init)();
        let view = unsafe {
            LayerHost::from_core_animation_layer(layer, &spec, state, (width, height), scale, dark, Arc::new(move || ring.ring()))
        };
        match view {
            Ok(view) => Box::into_raw(Box::new(ReaderHost { view, wake, insets: [0.0; 4] })),
            Err(e) => {
                eprintln!("reader_host_new: {e}");
                std::ptr::null_mut()
            }
        }
    })
}

/// # Safety
/// `host` came from `reader_host_new` and is not used again.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reader_host_free(host: *mut ReaderHost) {
    if host.is_null() {
        return;
    }
    guarded((), || {
        let host = unsafe { Box::from_raw(host) };
        host.wake.clear();
        drop(host);
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn reader_host_resize(host: *mut ReaderHost, width: f32, height: f32, scale: f32) {
    with_host(host, (), |h| h.view.resize((width, height), scale));
}

#[unsafe(no_mangle)]
pub extern "C" fn reader_host_set_insets(host: *mut ReaderHost, top: f32, right: f32, bottom: f32, left: f32) {
    with_host(host, (), |h| {
        let insets = [top, right, bottom, left];
        if insets != h.insets {
            h.insets = insets;
            h.view.runtime_mut().update(|s, _| reader_app::set_insets(s, insets));
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn reader_host_set_dark(host: *mut ReaderHost, dark: bool) {
    with_host(host, (), |h| h.view.runtime_mut().set_system_dark(dark));
}

#[unsafe(no_mangle)]
pub extern "C" fn reader_host_frame(host: *mut ReaderHost) -> bool {
    with_host(host, false, |h| {
        let out = h.view.frame();
        if !out.animating
            && let Some(secs) = out.wake_after_secs
        {
            h.wake.ring_after(secs);
        }
        out.animating
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn reader_host_touch(host: *mut ReaderHost, id: u64, phase: u32, x: f32, y: f32, time: f64) {
    let phase = match phase {
        0 => TouchPhase::Started,
        1 => TouchPhase::Moved,
        2 => TouchPhase::Ended,
        3 => TouchPhase::Cancelled,
        _ => return,
    };
    with_host(host, (), |h| h.view.runtime_mut().touch(id, phase, x, y, time));
}

#[unsafe(no_mangle)]
pub extern "C" fn reader_host_take_request(host: *mut ReaderHost) -> u32 {
    with_host(host, REQUEST_NONE, |h| match reader_app::take_request(&mut h.view.runtime_mut().state) {
        Some(HostRequest::PickBook) => REQUEST_PICK_BOOK,
        Some(HostRequest::FindBooks) => REQUEST_FIND_BOOKS,
        Some(HostRequest::AddFolder) => REQUEST_ADD_FOLDER,
        Some(HostRequest::ForgetFolders) => REQUEST_FORGET_FOLDERS,
        Some(HostRequest::AskCredentials { create: false }) => REQUEST_SIGN_IN,
        Some(HostRequest::AskCredentials { create: true }) => REQUEST_CREATE_ACCOUNT,
        None => REQUEST_NONE,
    })
}

/// # Safety
/// `path` is a NUL-terminated path inside the app's sandbox.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reader_host_import(host: *mut ReaderHost, path: *const c_char) {
    let Some(path) = path_arg(path) else { return };
    with_host(host, (), |h| h.view.runtime_mut().update(|s, _| reader_app::import(s, &path)));
}

/// # Safety
/// `paths` points to `count` NUL-terminated paths the host keeps readable until the next search.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reader_host_search_folders(host: *mut ReaderHost, paths: *const *const c_char, count: usize) {
    if paths.is_null() {
        return;
    }
    let paths: Vec<PathBuf> = unsafe { std::slice::from_raw_parts(paths, count) }.iter().filter_map(|p| path_arg(*p)).collect();
    with_host(host, (), |h| h.view.runtime_mut().update(|s, _| reader_app::search_folders(s, paths)));
}

/// # Safety
/// `email` and `password` are NUL-terminated UTF-8 strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reader_host_submit_credentials(host: *mut ReaderHost, email: *const c_char, password: *const c_char, create: bool) {
    if email.is_null() || password.is_null() {
        return;
    }
    let email = unsafe { CStr::from_ptr(email) }.to_string_lossy().trim().to_string();
    let password = unsafe { CStr::from_ptr(password) }.to_string_lossy().into_owned();
    with_host(host, (), |h| h.view.runtime_mut().update(|s, _| reader_app::submit_credentials(s, email, password, create)));
}

/// Whether the page is dark, read from the color the last frame cleared to (the root's fill).
#[unsafe(no_mangle)]
pub extern "C" fn reader_host_page_dark(host: *mut ReaderHost) -> bool {
    with_host(host, false, |h| {
        let [r, g, b, _] = h.view.runtime().clear_color();
        0.2126 * r + 0.7152 * g + 0.0722 * b < 0.5
    })
}

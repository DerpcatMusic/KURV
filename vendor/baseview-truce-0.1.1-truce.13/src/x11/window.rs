use std::cell::Cell;
use std::error::Error;
use std::ffi::c_void;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use raw_window_handle::{
    HasRawDisplayHandle, HasRawWindowHandle, RawDisplayHandle, RawWindowHandle, XlibDisplayHandle,
    XlibWindowHandle,
};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ChangeWindowAttributesAux, ConfigureWindowAux, ConnectionExt as _, CreateGCAux,
    CreateWindowAux, EventMask, PropMode, Visualid, Window as XWindow, WindowClass,
};
use x11rb::wrapper::ConnectionExt as _;

use super::XcbConnection;
use crate::{
    Event, MouseCursor, Size, WindowEvent, WindowHandler, WindowInfo, WindowOpenOptions,
    WindowScalePolicy,
};

#[cfg(feature = "opengl")]
use crate::gl::{platform, GlContext};
use crate::x11::event_loop::EventLoop;
use crate::x11::visual_info::WindowVisualConfig;

pub struct WindowHandle {
    raw_window_handle: Option<RawWindowHandle>,
    event_loop_handle: Option<JoinHandle<()>>,
    close_requested: Arc<AtomicBool>,
    callback_phase: Arc<AtomicU8>,
    allow_bounded_close_detach: bool,
    is_open: Arc<AtomicBool>,
}

// A normal event-loop turn is 15 ms. This leaves ample time for a healthy
// thread to observe shutdown while preventing a wedged Vulkan/X11 present from
// blocking the DAW's host thread forever.
const CLOSE_JOIN_TIMEOUT: Duration = Duration::from_millis(250);
const CLOSE_JOIN_POLL: Duration = Duration::from_millis(1);
const CALLBACK_IDLE: u8 = 0;
const CALLBACK_ACTIVE: u8 = 1;
const CALLBACK_REVOKED: u8 = 2;

pub(crate) struct CallbackGuard(Arc<AtomicU8>);

impl Drop for CallbackGuard {
    fn drop(&mut self) {
        // A timeout may have changed ACTIVE to REVOKED while the callback
        // was blocked. Never overwrite that terminal state on return.
        let _ = self.0.compare_exchange(
            CALLBACK_ACTIVE,
            CALLBACK_IDLE,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
}

impl WindowHandle {
    pub fn close(&mut self) {
        self.close_requested.store(true, Ordering::Release);
        if let Some(event_loop) = self.event_loop_handle.take() {
            join_event_loop_bounded(
                event_loop,
                CLOSE_JOIN_TIMEOUT,
                Arc::clone(&self.callback_phase),
                self.allow_bounded_close_detach,
            );
        }
    }

    pub fn is_open(&self) -> bool {
        self.is_open.load(Ordering::Acquire)
    }
}

fn join_event_loop_bounded(
    event_loop: JoinHandle<()>, timeout: Duration, callback_phase: Arc<AtomicU8>,
    allow_detach: bool,
) {
    join_event_loop_bounded_with(
        event_loop,
        timeout,
        callback_phase,
        allow_detach,
        pin_current_image,
    );
}

fn join_event_loop_bounded_with(
    event_loop: JoinHandle<()>, timeout: Duration, callback_phase: Arc<AtomicU8>,
    allow_detach: bool, pin_before_detach: impl FnOnce() -> bool,
) {
    let deadline = Instant::now() + timeout;
    while !event_loop.is_finished() {
        let now = Instant::now();
        if now >= deadline {
            // Revoke handler callbacks before attempting to detach. If the
            // wedged call later returns then the event loop exits without
            // re-entering plug-in/host-facing handler code.
            callback_phase.store(CALLBACK_REVOKED, Ordering::Release);
            // A detached thread may resume after the host unloads the plug-in.
            // Keep the containing ELF image mapped first. If that cannot be
            // guaranteed, retain the old synchronous join rather than trade a
            // hang for execution through unmapped code.
            if allow_detach && pin_before_detach() {
                return;
            }
            let _ = event_loop.join();
            return;
        }
        thread::sleep(CLOSE_JOIN_POLL.min(deadline.saturating_duration_since(now)));
    }
    let _ = event_loop.join();
}

/// Pin the current ELF image before intentionally detaching bounded background work.
///
/// The returned mapping is process-lifetime by design. This uses the same
/// audited `RTLD_NODELETE` policy as the X11 bounded-close fallback.
pub fn pin_current_image_for_detached_work() -> bool {
    static PINNED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *PINNED.get_or_init(pin_current_image)
}

fn pin_current_image() -> bool {
    // RTLD_NODELETE is part of glibc and musl's Linux ABI but libc does not
    // expose the named constant on every Linux libc target.
    const RTLD_NODELETE_FLAG: libc::c_int = 0x1000;
    let mut info = std::mem::MaybeUninit::<libc::Dl_info>::zeroed();
    let symbol = join_event_loop_bounded as *const () as *const libc::c_void;
    // SAFETY: `info` points to writable storage and `symbol` is an address in
    // the currently executing image. `dli_fname` is owned by the loader and is
    // valid for the immediately following `dlopen` call.
    let found = unsafe { libc::dladdr(symbol, info.as_mut_ptr()) };
    if found == 0 {
        return false;
    }
    // SAFETY: successful `dladdr` initialized `info`; a null filename is
    // explicitly rejected. The handle is intentionally leaked and NODELETE is
    // intentional: it pins code a detached graphics thread may return into.
    let info = unsafe { info.assume_init() };
    if info.dli_fname.is_null() {
        return false;
    }
    unsafe {
        !libc::dlopen(info.dli_fname, libc::RTLD_NOW | libc::RTLD_LOCAL | RTLD_NODELETE_FLAG)
            .is_null()
    }
}

unsafe impl HasRawWindowHandle for WindowHandle {
    fn raw_window_handle(&self) -> RawWindowHandle {
        if let Some(raw_window_handle) = self.raw_window_handle {
            if self.is_open.load(Ordering::Acquire) {
                return raw_window_handle;
            }
        }

        RawWindowHandle::Xlib(XlibWindowHandle::empty())
    }
}

pub(crate) struct ParentHandle {
    close_requested: Arc<AtomicBool>,
    callback_phase: Arc<AtomicU8>,
    is_open: Arc<AtomicBool>,
}

impl ParentHandle {
    pub fn new(allow_bounded_close_detach: bool) -> (Self, WindowHandle) {
        let close_requested = Arc::new(AtomicBool::new(false));
        let callback_phase = Arc::new(AtomicU8::new(CALLBACK_IDLE));
        let is_open = Arc::new(AtomicBool::new(true));
        let handle = WindowHandle {
            raw_window_handle: None,
            event_loop_handle: None,
            close_requested: Arc::clone(&close_requested),
            callback_phase: Arc::clone(&callback_phase),
            allow_bounded_close_detach,
            is_open: Arc::clone(&is_open),
        };

        (Self { close_requested, callback_phase, is_open }, handle)
    }

    pub fn parent_did_drop(&self) -> bool {
        self.close_requested.load(Ordering::Acquire)
    }

    pub fn begin_callback(&self) -> Option<CallbackGuard> {
        self.callback_phase
            .compare_exchange(CALLBACK_IDLE, CALLBACK_ACTIVE, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| CallbackGuard(Arc::clone(&self.callback_phase)))
    }
}

impl Drop for ParentHandle {
    fn drop(&mut self) {
        self.is_open.store(false, Ordering::Release);
    }
}

pub(crate) struct WindowInner {
    // GlContext should be dropped **before** XcbConnection is dropped
    #[cfg(feature = "opengl")]
    gl_context: Option<GlContext>,

    pub(crate) xcb_connection: Rc<XcbConnection>,
    pub(crate) window_id: XWindow,
    /// The host-supplied parent window when this window is embedded
    /// (`open_parented`). Some DAWs (Bitwig on Linux) resize the embed
    /// parent directly instead of driving the plugin's resize API, so we
    /// watch its `ConfigureNotify` and size our child to fill it. `None`
    /// for top-level windows (parented to the root).
    pub(crate) embed_parent_id: Option<XWindow>,
    pub(crate) window_info: WindowInfo,
    visual_id: Visualid,
    mouse_cursor: Cell<MouseCursor>,

    pub(crate) close_requested: Cell<bool>,

    /// New content-scale factor requested via [`Window::set_scale_factor`]
    /// since the last event-loop drain. Set from the handler (which only holds
    /// a shared `&WindowInner`), consumed by the event loop when it applies the
    /// resulting `ConfigureNotify` so the new physical size is converted to
    /// logical at the new scale rather than the stale creation-time scale.
    pub(crate) pending_scale: Cell<Option<f64>>,
}

pub struct Window<'a> {
    pub(crate) inner: &'a WindowInner,
}

// Hack to allow sending a RawWindowHandle between threads. Do not make public
struct SendableRwh(RawWindowHandle);

unsafe impl Send for SendableRwh {}

type WindowOpenResult = Result<SendableRwh, ()>;

impl<'a> Window<'a> {
    pub fn open_parented<P, H, B>(parent: &P, options: WindowOpenOptions, build: B) -> WindowHandle
    where
        P: HasRawWindowHandle,
        H: WindowHandler + 'static,
        B: FnOnce(&mut crate::Window) -> H,
        B: Send + 'static,
    {
        // Convert parent into something that X understands
        let parent_id = match parent.raw_window_handle() {
            RawWindowHandle::Xlib(h) => h.window as u32,
            RawWindowHandle::Xcb(h) => h.window,
            h => panic!("unsupported parent handle type {:?}", h),
        };

        let (tx, rx) = mpsc::sync_channel::<WindowOpenResult>(1);
        let (parent_handle, mut window_handle) = ParentHandle::new(H::allow_bounded_close_detach());
        let join_handle = thread::spawn(move || {
            Self::window_thread(Some(parent_id), options, build, tx.clone(), Some(parent_handle))
                .unwrap();
        });

        let raw_window_handle = rx.recv().unwrap().unwrap();
        window_handle.raw_window_handle = Some(raw_window_handle.0);
        window_handle.event_loop_handle = Some(join_handle);
        window_handle
    }

    pub fn open_blocking<H, B>(options: WindowOpenOptions, build: B)
    where
        H: WindowHandler + 'static,
        B: FnOnce(&mut crate::Window) -> H,
        B: Send + 'static,
    {
        let (tx, rx) = mpsc::sync_channel::<WindowOpenResult>(1);

        let thread = thread::spawn(move || {
            Self::window_thread(None, options, build, tx, None).unwrap();
        });

        let _ = rx.recv().unwrap().unwrap();

        thread.join().unwrap_or_else(|err| {
            eprintln!("Window thread panicked: {:#?}", err);
        });
    }

    fn window_thread<H, B>(
        parent: Option<u32>, options: WindowOpenOptions, build: B,
        tx: mpsc::SyncSender<WindowOpenResult>, parent_handle: Option<ParentHandle>,
    ) -> Result<(), Box<dyn Error>>
    where
        H: WindowHandler + 'static,
        B: FnOnce(&mut crate::Window) -> H,
        B: Send + 'static,
    {
        // Connect to the X server
        // FIXME: baseview error type instead of unwrap()
        let xcb_connection = XcbConnection::new()?;

        // Get screen information
        let screen = xcb_connection.screen();
        let parent_id = parent.unwrap_or(screen.root);

        let gc_id = xcb_connection.conn.generate_id()?;
        xcb_connection.conn.create_gc(
            gc_id,
            parent_id,
            &CreateGCAux::new().foreground(screen.black_pixel).graphics_exposures(0),
        )?;

        let scaling = match options.scale {
            WindowScalePolicy::SystemScaleFactor => xcb_connection.get_scaling().unwrap_or(1.0),
            WindowScalePolicy::ScaleFactor(scale) => scale,
        };

        let window_info = WindowInfo::from_logical_size(options.size, scaling);

        #[cfg(feature = "opengl")]
        let visual_info =
            WindowVisualConfig::find_best_visual_config_for_gl(&xcb_connection, options.gl_config)?;

        #[cfg(not(feature = "opengl"))]
        let visual_info = WindowVisualConfig::find_best_visual_config(&xcb_connection)?;

        let window_id = xcb_connection.conn.generate_id()?;
        xcb_connection.conn.create_window(
            visual_info.visual_depth,
            window_id,
            parent_id,
            0,                                         // x coordinate of the new window
            0,                                         // y coordinate of the new window
            window_info.physical_size().width as u16,  // window width
            window_info.physical_size().height as u16, // window height
            0,                                         // window border
            WindowClass::INPUT_OUTPUT,
            visual_info.visual_id,
            &CreateWindowAux::new()
                .event_mask(
                    EventMask::EXPOSURE
                        | EventMask::POINTER_MOTION
                        | EventMask::BUTTON_PRESS
                        | EventMask::BUTTON_RELEASE
                        | EventMask::KEY_PRESS
                        | EventMask::KEY_RELEASE
                        | EventMask::STRUCTURE_NOTIFY
                        | EventMask::ENTER_WINDOW
                        | EventMask::LEAVE_WINDOW
                        | EventMask::FOCUS_CHANGE,
                )
                // As mentioned above, these two values are needed to be able to create a window
                // with a depth of 32-bits when the parent window has a different depth
                .colormap(visual_info.color_map)
                .border_pixel(0),
        )?;
        xcb_connection.conn.map_window(window_id)?;

        // Change window title
        let title = options.title;
        xcb_connection.conn.change_property8(
            PropMode::REPLACE,
            window_id,
            AtomEnum::WM_NAME,
            AtomEnum::STRING,
            title.as_bytes(),
        )?;

        xcb_connection.conn.change_property32(
            PropMode::REPLACE,
            window_id,
            xcb_connection.atoms.WM_PROTOCOLS,
            AtomEnum::ATOM,
            &[xcb_connection.atoms.WM_DELETE_WINDOW],
        )?;

        // Advertise XDND on the actual content window, rather than only on a
        // top-level shell. This is important for `open_parented`: XDND source
        // clients walk the child hierarchy under the pointer and can then
        // target the embedded plug-in window directly without modifying or
        // stealing the host's own drag-and-drop properties.
        xcb_connection.conn.change_property32(
            PropMode::REPLACE,
            window_id,
            xcb_connection.atoms.XdndAware,
            AtomEnum::ATOM,
            &[crate::x11::dnd::XDND_VERSION],
        )?;

        xcb_connection.conn.flush()?;
        let xcb_connection = Rc::new(xcb_connection);

        // TODO: These APIs could use a couple tweaks now that everything is internal and there is
        //       no error handling anymore at this point. Everything is more or less unchanged
        //       compared to when raw-gl-context was a separate crate.
        #[cfg(feature = "opengl")]
        let gl_context = visual_info.fb_config.map(|fb_config| {
            use std::ffi::c_ulong;

            let window = window_id as c_ulong;

            // Because of the visual negotation we had to take some extra steps to create this context
            let context =
                platform::GlContext::create(window, Rc::clone(&xcb_connection), fb_config)
                    .expect("Could not create OpenGL context");
            GlContext::new(context)
        });

        // Watch the embed parent for size changes: some hosts (Bitwig on
        // Linux) resize the parent embed window directly rather than
        // calling the plugin's resize API, and X11 does not auto-resize
        // children. Selecting `STRUCTURE_NOTIFY` on the parent lets the
        // event loop mirror the parent's size onto our child. Best-effort:
        // ignore failures (e.g. a parent we may not select on).
        let embed_parent_id = parent;
        if let Some(pid) = embed_parent_id {
            let _ = xcb_connection.conn.change_window_attributes(
                pid,
                &ChangeWindowAttributesAux::new().event_mask(EventMask::STRUCTURE_NOTIFY),
            );
            let _ = xcb_connection.conn.flush();
        }

        let mut inner = WindowInner {
            xcb_connection,
            window_id,
            embed_parent_id,
            window_info,
            visual_id: visual_info.visual_id,
            mouse_cursor: Cell::new(MouseCursor::default()),

            close_requested: Cell::new(false),

            pending_scale: Cell::new(None),

            #[cfg(feature = "opengl")]
            gl_context,
        };

        let mut window = crate::Window::new(Window { inner: &mut inner });

        let mut handler = build(&mut window);

        // Send an initial window resized event so the user is alerted of
        // the correct dpi scaling.
        handler.on_event(&mut window, Event::Window(WindowEvent::Resized(window_info)));

        let _ = tx.send(Ok(SendableRwh(window.raw_window_handle())));

        EventLoop::new(inner, handler, parent_handle).run()?;

        Ok(())
    }

    pub fn set_mouse_cursor(&self, mouse_cursor: MouseCursor) {
        if self.inner.mouse_cursor.get() == mouse_cursor {
            return;
        }

        let xid = self.inner.xcb_connection.get_cursor(mouse_cursor).unwrap();

        if xid != 0 {
            let _ = self.inner.xcb_connection.conn.change_window_attributes(
                self.inner.window_id,
                &ChangeWindowAttributesAux::new().cursor(xid),
            );
            let _ = self.inner.xcb_connection.conn.flush();
        }

        self.inner.mouse_cursor.set(mouse_cursor);
    }

    pub fn close(&mut self) {
        self.inner.close_requested.set(true);
    }

    pub fn has_focus(&mut self) -> bool {
        unimplemented!()
    }

    pub fn focus(&mut self) {
        unimplemented!()
    }

    pub fn resize(&mut self, size: Size) {
        let scaling = self.inner.window_info.scale();
        let new_window_info = WindowInfo::from_logical_size(size, scaling);

        let _ = self.inner.xcb_connection.conn.configure_window(
            self.inner.window_id,
            &ConfigureWindowAux::new()
                .width(new_window_info.physical_size().width)
                .height(new_window_info.physical_size().height),
        );
        let _ = self.inner.xcb_connection.conn.flush();

        // This will trigger a `ConfigureNotify` event which will in turn change `self.window_info`
        // and notify the window handler about it
    }

    /// Re-interpret the window at a new content-scale factor.
    ///
    /// The *physical* pixel size is left untouched - for an embedded plug-in
    /// view that is the host's to control (it drives the embed parent, which we
    /// mirror onto the child) - and only the scale, and hence the derived
    /// logical size (`physical / scale`) and mouse-coordinate mapping, change.
    ///
    /// This exists because `WindowScalePolicy` is fixed at open, yet a host may
    /// report its content scale only *after* the view is attached (e.g. REAPER
    /// on Linux calling `IPlugViewContentScaleSupport::setContentScaleFactor`
    /// after `IPlugView::attached`). Without a way to update it, the child
    /// stays at the creation-time scale: it renders 1x content in a 2x frame
    /// and mouse coordinates are divided by the stale factor.
    ///
    /// The change is applied by the event loop on its next drain, which then
    /// notifies the handler with a `Resized` carrying the new scale. A
    /// non-finite or non-positive `scale`, or one equal to the current scale,
    /// is a no-op.
    pub fn set_scale_factor(&mut self, scale: f64) {
        let cur = self.inner.window_info.scale();
        if !scale.is_finite() || scale <= 0.0 || (scale - cur).abs() < f64::EPSILON {
            return;
        }
        // We only hold a shared `&WindowInner` here, so we can't rewrite
        // `window_info` directly. Stash the new scale; the event loop applies
        // it against the current physical size on its next drain and emits the
        // `Resized`. Do NOT resize the child - re-scaling must not fight the
        // host's own sizing of the embed parent.
        self.inner.pending_scale.set(Some(scale));
    }

    #[cfg(feature = "opengl")]
    pub fn gl_context(&self) -> Option<&crate::gl::GlContext> {
        self.inner.gl_context.as_ref()
    }
}

unsafe impl<'a> HasRawWindowHandle for Window<'a> {
    fn raw_window_handle(&self) -> RawWindowHandle {
        let mut handle = XlibWindowHandle::empty();

        handle.window = self.inner.window_id.into();
        handle.visual_id = self.inner.visual_id.into();

        RawWindowHandle::Xlib(handle)
    }
}

unsafe impl<'a> HasRawDisplayHandle for Window<'a> {
    fn raw_display_handle(&self) -> RawDisplayHandle {
        let display = self.inner.xcb_connection.conn.xlib_display();
        let mut handle = XlibDisplayHandle::empty();

        handle.display = display as *mut c_void;
        handle.screen = self.inner.xcb_connection.conn.default_screen();

        RawDisplayHandle::Xlib(handle)
    }
}

pub fn copy_to_clipboard(_data: &str) {
    todo!()
}

#[cfg(test)]
mod close_tests {
    use super::*;

    #[test]
    fn bounded_join_waits_for_a_healthy_event_loop() {
        let event_loop = thread::spawn(|| {});
        join_event_loop_bounded_with(
            event_loop,
            Duration::from_millis(100),
            Arc::new(AtomicU8::new(CALLBACK_IDLE)),
            false,
            || false,
        );
    }

    #[test]
    fn bounded_join_detaches_only_after_lifetime_pin_succeeds() {
        let (release_tx, release_rx) = mpsc::channel();
        let (finished_tx, finished_rx) = mpsc::channel();
        let event_loop = thread::spawn(move || {
            let _ = release_rx.recv();
            let _ = finished_tx.send(());
        });
        let callback_phase = Arc::new(AtomicU8::new(CALLBACK_IDLE));
        join_event_loop_bounded_with(
            event_loop,
            Duration::ZERO,
            Arc::clone(&callback_phase),
            true,
            || true,
        );
        assert_eq!(callback_phase.load(Ordering::Acquire), CALLBACK_REVOKED);
        release_tx.send(()).expect("release detached event loop");
        finished_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("detached event loop finishes after release");
    }

    #[test]
    fn callback_revocation_is_terminal() {
        let (parent, handle) = ParentHandle::new(true);
        let callback = parent.begin_callback().expect("initial callback guard");
        handle.callback_phase.store(CALLBACK_REVOKED, Ordering::Release);
        drop(callback);
        assert_eq!(handle.callback_phase.load(Ordering::Acquire), CALLBACK_REVOKED);
        assert!(parent.begin_callback().is_none());
    }

    #[test]
    fn bounded_join_remains_synchronous_when_lifetime_pin_fails() {
        let (release_tx, release_rx) = mpsc::channel();
        let (join_returned_tx, join_returned_rx) = mpsc::channel();
        let callback_phase = Arc::new(AtomicU8::new(CALLBACK_IDLE));
        let phase_for_join = Arc::clone(&callback_phase);
        let join_caller = thread::spawn(move || {
            let event_loop = thread::spawn(move || {
                let _ = release_rx.recv();
            });
            join_event_loop_bounded_with(event_loop, Duration::ZERO, phase_for_join, true, || {
                false
            });
            let _ = join_returned_tx.send(());
        });
        assert!(join_returned_rx.recv_timeout(Duration::from_millis(20)).is_err());
        assert_eq!(callback_phase.load(Ordering::Acquire), CALLBACK_REVOKED);
        release_tx.send(()).expect("release synchronously joined loop");
        join_returned_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("join returns once the event loop finishes");
        join_caller.join().expect("join caller");
    }

    #[test]
    fn bounded_detach_is_opt_in_for_audited_handlers() {
        let pin_called = Arc::new(AtomicBool::new(false));
        let pin_called_in_join = Arc::clone(&pin_called);
        let event_loop = thread::spawn(|| thread::sleep(Duration::from_millis(2)));

        join_event_loop_bounded_with(
            event_loop,
            Duration::ZERO,
            Arc::new(AtomicU8::new(CALLBACK_IDLE)),
            false,
            move || {
                pin_called_in_join.store(true, Ordering::Release);
                true
            },
        );

        assert!(!pin_called.load(Ordering::Acquire));
    }
}

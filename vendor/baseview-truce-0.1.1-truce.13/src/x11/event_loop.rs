use crate::x11::dnd::{
    finished_payload, parse_enter_flags, parse_file_list, status_flags, unpack_root_coordinates,
    DragPosition, FileListKind, XdndSelectionRequest, XdndState, MAX_TYPE_LIST_ATOMS,
    MAX_URI_LIST_BYTES,
};
use crate::x11::keyboard::{convert_key_press_event, convert_key_release_event, key_mods};
use crate::x11::{ParentHandle, Window, WindowInner};
use crate::{
    DropData, DropEffect, Event, EventStatus, MouseButton, MouseEvent, PhyPoint, PhySize,
    ScrollDelta, WindowEvent, WindowHandler, WindowInfo,
};
use std::error::Error;
use std::os::fd::AsRawFd;
use std::time::{Duration, Instant};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ClientMessageEvent, ConfigureWindowAux, ConnectionExt as _, EventMask,
    SelectionNotifyEvent, Timestamp, Window as XWindow,
};
use x11rb::protocol::Event as XEvent;
use x11rb::CURRENT_TIME;

pub(super) struct EventLoop {
    handler: Box<dyn WindowHandler>,
    window: WindowInner,
    parent_handle: Option<ParentHandle>,

    new_physical_size: Option<PhySize>,
    frame_interval: Duration,
    event_loop_running: bool,
    xdnd: XdndState,
}

impl EventLoop {
    pub fn new(
        window: WindowInner, handler: impl WindowHandler + 'static,
        parent_handle: Option<ParentHandle>,
    ) -> Self {
        Self {
            window,
            handler: Box::new(handler),
            parent_handle,
            frame_interval: Duration::from_millis(15),
            event_loop_running: false,
            new_physical_size: None,
            xdnd: XdndState::default(),
        }
    }

    #[inline]
    fn drain_xcb_events(&mut self) -> Result<(), Box<dyn Error>> {
        // the X server has a tendency to send spurious/extraneous configure notify events when a
        // window is resized, and we need to batch those together and just send one resize event
        // when they've all been coalesced.
        self.new_physical_size = None;

        while let Some(event) = self.window.xcb_connection.conn.poll_for_event()? {
            self.handle_xcb_event(event);
        }

        // A `set_scale_factor` call stashes the new content scale here; apply
        // it to whatever physical size the coalesced `ConfigureNotify`s settled
        // on (or, if the physical size didn't change, the current one) so the
        // logical size and reported scale reflect the host's new factor.
        let pending_scale = self.window.pending_scale.take();
        if self.new_physical_size.is_some() || pending_scale.is_some() {
            let size = self
                .new_physical_size
                .take()
                .unwrap_or_else(|| self.window.window_info.physical_size());
            let scale = pending_scale.unwrap_or_else(|| self.window.window_info.scale());

            self.window.window_info = WindowInfo::from_physical_size(size, scale);

            let window_info = self.window.window_info;

            self.handle_event(Event::Window(WindowEvent::Resized(window_info)));
        }

        Ok(())
    }

    // Event loop
    // FIXME: poll() acts fine on linux, sometimes funky on *BSD. XCB upstream uses a define to
    // switch between poll() and select() (the latter of which is fine on *BSD), and we should do
    // the same.
    pub fn run(&mut self) -> Result<(), Box<dyn Error>> {
        use nix::poll::*;

        let xcb_fd = self.window.xcb_connection.conn.as_raw_fd();

        let mut last_frame = Instant::now();
        self.event_loop_running = true;

        while self.event_loop_running {
            // Observe cross-thread shutdown before entering user drawing or a
            // potentially blocking graphics-driver call. The check below the
            // poll remains necessary for requests arriving while asleep.
            if self.parent_handle.as_ref().is_some_and(ParentHandle::parent_did_drop) {
                self.handle_must_close();
                continue;
            }

            // We'll try to keep a consistent frame pace. If the last frame couldn't be processed in
            // the expected frame time, this will throttle down to prevent multiple frames from
            // being queued up. The conditional here is needed because event handling and frame
            // drawing is interleaved. The `poll()` function below will wait until the next frame
            // can be drawn, or until the window receives an event. We thus need to manually check
            // if it's already time to draw a new frame.
            let next_frame = last_frame + self.frame_interval;
            if Instant::now() >= next_frame {
                let callback = self.parent_handle.as_ref().and_then(ParentHandle::begin_callback);
                if self.parent_handle.is_none() || callback.is_some() {
                    self.handler.on_frame(&mut crate::Window::new(Window { inner: &self.window }));
                }
                drop(callback);
                last_frame = Instant::max(next_frame, Instant::now() - self.frame_interval);
            }

            let mut fds = [PollFd::new(xcb_fd, PollFlags::POLLIN)];

            // Check for any events in the internal buffers
            // before going to sleep:
            self.drain_xcb_events()?;

            // FIXME: handle errors
            poll(&mut fds, next_frame.duration_since(Instant::now()).subsec_millis() as i32)
                .unwrap();

            if let Some(revents) = fds[0].revents() {
                if revents.contains(PollFlags::POLLERR) {
                    panic!("xcb connection poll error");
                }

                if revents.contains(PollFlags::POLLIN) {
                    self.drain_xcb_events()?;
                }
            }

            // Check if the parents's handle was dropped (such as when the host
            // requested the window to close)
            if let Some(parent_handle) = &self.parent_handle {
                if parent_handle.parent_did_drop() {
                    self.handle_must_close();
                    self.window.close_requested.set(false);
                }
            }

            // Check if the user has requested the window to close
            if self.window.close_requested.get() {
                self.handle_must_close();
                self.window.close_requested.set(false);
            }
        }

        Ok(())
    }

    fn handle_xcb_event(&mut self, event: XEvent) {
        // For all the keyboard and mouse events, you can fetch
        // `x`, `y`, `detail`, and `state`.
        // - `x` and `y` are the position inside the window where the cursor currently is
        //   when the event happened.
        // - `detail` will tell you which keycode was pressed/released (for keyboard events)
        //   or which mouse button was pressed/released (for mouse events).
        //   For mouse events, here's what the value means (at least on my current mouse):
        //      1 = left mouse button
        //      2 = middle mouse button (scroll wheel)
        //      3 = right mouse button
        //      4 = scroll wheel up
        //      5 = scroll wheel down
        //      8 = lower side button ("back" button)
        //      9 = upper side button ("forward" button)
        //   Note that you *will* get a "button released" event for even the scroll wheel
        //   events, which you can probably ignore.
        // - `state` will tell you the state of the main three mouse buttons and some of
        //   the keyboard modifier keys at the time of the event.
        //   http://rtbo.github.io/rust-xcb/src/xcb/ffi/xproto.rs.html#445

        match event {
            ////
            // window
            ////
            XEvent::ClientMessage(event)
                if event.format == 32 && self.handle_xdnd_client_message(&event) => {}

            XEvent::ClientMessage(event)
                if event.format == 32
                    && event.data.as_data32()[0]
                        == self.window.xcb_connection.atoms.WM_DELETE_WINDOW =>
            {
                self.handle_close_requested();
            }

            XEvent::SelectionNotify(event) => {
                self.handle_xdnd_selection(event);
            }

            XEvent::ConfigureNotify(event) => {
                // The embed parent was resized (Bitwig on Linux resizes the
                // parent directly rather than calling the plugin's resize
                // API). X11 does not auto-resize children, so mirror the
                // parent's new size onto our child; the child's own
                // ConfigureNotify (below) then drives the resize event.
                if Some(event.window) == self.window.embed_parent_id {
                    let pw = event.width as u32;
                    let ph = event.height as u32;
                    let cur = self.window.window_info.physical_size();
                    if pw > 0 && ph > 0 && (pw != cur.width || ph != cur.height) {
                        let _ = self.window.xcb_connection.conn.configure_window(
                            self.window.window_id,
                            &ConfigureWindowAux::new().width(pw).height(ph),
                        );
                        let _ = self.window.xcb_connection.conn.flush();
                    }
                    return;
                }

                let new_physical_size = PhySize::new(event.width as u32, event.height as u32);

                if self.new_physical_size.is_some()
                    || new_physical_size != self.window.window_info.physical_size()
                {
                    self.new_physical_size = Some(new_physical_size);
                }
            }

            ////
            // mouse
            ////
            XEvent::MotionNotify(event) => {
                let physical_pos = PhyPoint::new(event.event_x as i32, event.event_y as i32);
                let logical_pos = physical_pos.to_logical(&self.window.window_info);

                self.handle_event(Event::Mouse(MouseEvent::CursorMoved {
                    position: logical_pos,
                    modifiers: key_mods(event.state),
                }));
            }

            XEvent::EnterNotify(event) => {
                self.handle_event(Event::Mouse(MouseEvent::CursorEntered));
                // since no `MOTION_NOTIFY` event is generated when `ENTER_NOTIFY` is generated,
                // we generate a CursorMoved as well, so the mouse position from here isn't lost
                let physical_pos = PhyPoint::new(event.event_x as i32, event.event_y as i32);
                let logical_pos = physical_pos.to_logical(&self.window.window_info);
                self.handle_event(Event::Mouse(MouseEvent::CursorMoved {
                    position: logical_pos,
                    modifiers: key_mods(event.state),
                }));
            }

            XEvent::LeaveNotify(_) => {
                self.handle_event(Event::Mouse(MouseEvent::CursorLeft));
            }

            XEvent::ButtonPress(event) => match event.detail {
                4..=7 => {
                    self.handle_event(Event::Mouse(MouseEvent::WheelScrolled {
                        delta: match event.detail {
                            4 => ScrollDelta::Lines { x: 0.0, y: 1.0 },
                            5 => ScrollDelta::Lines { x: 0.0, y: -1.0 },
                            6 => ScrollDelta::Lines { x: -1.0, y: 0.0 },
                            7 => ScrollDelta::Lines { x: 1.0, y: 0.0 },
                            _ => unreachable!(),
                        },
                        modifiers: key_mods(event.state),
                    }));
                }
                detail => {
                    let button_id = mouse_id(detail);
                    self.handle_event(Event::Mouse(MouseEvent::ButtonPressed {
                        button: button_id,
                        modifiers: key_mods(event.state),
                    }));
                }
            },

            XEvent::ButtonRelease(event) if !(4..=7).contains(&event.detail) => {
                let button_id = mouse_id(event.detail);
                self.handle_event(Event::Mouse(MouseEvent::ButtonReleased {
                    button: button_id,
                    modifiers: key_mods(event.state),
                }));
            }

            ////
            // keys
            ////
            XEvent::KeyPress(event) => {
                self.handle_event(Event::Keyboard(convert_key_press_event(&event)));
            }

            XEvent::KeyRelease(event) => {
                self.handle_event(Event::Keyboard(convert_key_release_event(&event)));
            }

            XEvent::FocusIn(_) => {
                self.handle_event(Event::Window(WindowEvent::Focused));
            }

            XEvent::FocusOut(_) => {
                self.handle_event(Event::Window(WindowEvent::Unfocused));
            }

            _ => {}
        }
    }

    fn handle_xdnd_client_message(&mut self, event: &ClientMessageEvent) -> bool {
        let atoms = &self.window.xcb_connection.atoms;
        if event.type_ == atoms.XdndEnter {
            self.handle_xdnd_enter(event.data.as_data32());
        } else if event.type_ == atoms.XdndPosition {
            self.handle_xdnd_position(event.data.as_data32());
        } else if event.type_ == atoms.XdndLeave {
            self.handle_xdnd_leave(event.data.as_data32()[0]);
        } else if event.type_ == atoms.XdndDrop {
            self.handle_xdnd_drop(event.data.as_data32());
        } else {
            return false;
        }
        true
    }

    fn handle_xdnd_enter(&mut self, data: [u32; 5]) {
        if self.xdnd.entered {
            self.handle_event(Event::Mouse(MouseEvent::DragLeft));
        }

        let source = data[0];
        let flags = parse_enter_flags(data[1]);
        let offered_types = if flags.has_type_list {
            self.read_xdnd_type_list(source).unwrap_or_default()
        } else {
            data[2..].iter().copied().filter(|atom| *atom != 0).collect()
        };
        let selected_target = self.preferred_file_target(&offered_types);
        self.xdnd.begin(source, flags.version, selected_target.is_some(), selected_target);
    }

    fn preferred_file_target(&self, offered: &[Atom]) -> Option<Atom> {
        let atoms = &self.window.xcb_connection.atoms;
        let priority = [
            atoms.text_uri_list,
            atoms.text_uri_list_utf8,
            atoms.text_x_uri,
            atoms.application_x_kde4_urilist,
            atoms.x_special_gnome_copied_files,
            atoms.text_plain_utf8,
            atoms.text_plain,
            atoms.UTF8_STRING,
            u32::from(AtomEnum::STRING),
        ];
        priority.into_iter().find(|atom| offered.contains(atom))
    }

    fn file_list_kind(&self, target: Atom) -> Option<FileListKind> {
        let atoms = &self.window.xcb_connection.atoms;
        if target == atoms.text_uri_list
            || target == atoms.text_uri_list_utf8
            || target == atoms.text_x_uri
            || target == atoms.application_x_kde4_urilist
        {
            Some(FileListKind::UriList)
        } else if target == atoms.x_special_gnome_copied_files {
            Some(FileListKind::GnomeCopiedFiles)
        } else if target == atoms.text_plain
            || target == atoms.text_plain_utf8
            || target == atoms.UTF8_STRING
            || target == u32::from(AtomEnum::STRING)
        {
            Some(FileListKind::PlainPaths)
        } else {
            None
        }
    }

    fn read_xdnd_type_list(&self, source: XWindow) -> Option<Vec<Atom>> {
        let conn = &self.window.xcb_connection.conn;
        let atoms = &self.window.xcb_connection.atoms;
        let reply = conn
            .get_property(false, source, atoms.XdndTypeList, AtomEnum::ATOM, 0, MAX_TYPE_LIST_ATOMS)
            .ok()?
            .reply()
            .ok()?;
        if reply.type_ != u32::from(AtomEnum::ATOM) || reply.bytes_after != 0 {
            return None;
        }
        let atoms = reply.value32()?.filter(|atom| *atom != 0).collect();
        Some(atoms)
    }

    fn handle_xdnd_position(&mut self, data: [u32; 5]) {
        let source = data[0];
        if self.xdnd.source != Some(source) {
            self.send_xdnd_status(source, None);
            return;
        }
        if !self.xdnd.supports_uri_list {
            self.send_xdnd_status(source, None);
            return;
        }

        if let Some(position) = self.drag_position(data[2]) {
            self.xdnd.position = Some(position);
        }

        if self.xdnd.data.is_some() && self.xdnd.position.is_some() {
            self.dispatch_drag_update();
            self.send_xdnd_status(source, self.xdnd.effect);
            return;
        }

        let time = if self.xdnd.version >= 1 { data[3] } else { CURRENT_TIME };
        self.request_xdnd_selection(time);

        // Selection conversion is asynchronous. The offered URI type is safe
        // to accept provisionally so a source does not cancel the drag before
        // SelectionNotify lets the application make the final decision. A
        // second status with the handler's chosen effect is sent immediately
        // after the selection arrives.
        let provisional = self.effect_for_action(data[4]).or(Some(DropEffect::Copy));
        self.send_xdnd_status(source, provisional);
    }

    fn drag_position(&self, packed: u32) -> Option<DragPosition> {
        let (root_x, root_y) = unpack_root_coordinates(packed);
        let conn = &self.window.xcb_connection.conn;
        let translated = conn
            .translate_coordinates(
                self.window.xcb_connection.screen().root,
                self.window.window_id,
                root_x,
                root_y,
            )
            .ok()?
            .reply()
            .ok()?;
        let physical = PhyPoint::new(i32::from(translated.dst_x), i32::from(translated.dst_y));
        let modifiers = conn
            .query_pointer(self.window.window_id)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map(|reply| key_mods(reply.mask))
            .unwrap_or_default();
        Some(DragPosition { position: physical.to_logical(&self.window.window_info), modifiers })
    }

    fn request_xdnd_selection(&mut self, time: Timestamp) {
        if self.xdnd.selection_request.is_some() || self.xdnd.selection_failed {
            return;
        }
        let Some(source) = self.xdnd.source else {
            self.xdnd.selection_failed = true;
            return;
        };
        let atoms = &self.window.xcb_connection.atoms;
        let conn = &self.window.xcb_connection.conn;
        let owner_is_source = conn
            .get_selection_owner(atoms.XdndSelection)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .is_some_and(|reply| reply.owner == source);
        if !owner_is_source {
            self.xdnd.selection_failed = true;
            return;
        }

        let Some(target) = self.xdnd.selected_target else {
            self.xdnd.selection_failed = true;
            return;
        };
        let request = XdndSelectionRequest {
            source,
            requestor: self.window.window_id,
            selection: atoms.XdndSelection,
            target,
            property: atoms.XdndSelection,
            time,
        };
        if conn
            .convert_selection(
                request.requestor,
                request.selection,
                request.target,
                request.property,
                request.time,
            )
            .is_ok()
        {
            self.xdnd.selection_request = Some(request);
            let _ = conn.flush();
        } else {
            self.xdnd.selection_failed = true;
        }
    }

    fn handle_xdnd_selection(&mut self, event: SelectionNotifyEvent) {
        if self.xdnd.take_matching_selection_request(&event).is_none() {
            return;
        }

        let files = if event.property == 0 {
            None
        } else {
            self.read_xdnd_selection(event.property).filter(|files| !files.is_empty())
        };
        let Some(files) = files else {
            self.xdnd.selection_failed = true;
            self.reject_or_finish_xdnd();
            return;
        };

        self.xdnd.data = Some(DropData::Files(files));
        if self.xdnd.position.is_some() {
            self.dispatch_drag_update();
        }

        if self.xdnd.pending_drop {
            self.complete_xdnd_drop();
        } else if let Some(source) = self.xdnd.source {
            self.send_xdnd_status(source, self.xdnd.effect);
        }
    }

    fn read_xdnd_selection(&self, property: Atom) -> Option<Vec<std::path::PathBuf>> {
        let conn = &self.window.xcb_connection.conn;
        let reply = conn
            .get_property(
                true,
                self.window.window_id,
                property,
                AtomEnum::ANY,
                0,
                (MAX_URI_LIST_BYTES / 4) as u32,
            )
            .ok()?
            .reply()
            .ok()?;
        let kind = self
            .file_list_kind(reply.type_)
            .or_else(|| self.xdnd.selected_target.and_then(|target| self.file_list_kind(target)))?;
        if reply.format != 8 || reply.bytes_after != 0 || reply.value.len() > MAX_URI_LIST_BYTES {
            return None;
        }
        let files = parse_file_list(kind, &reply.value);
        (!files.is_empty()).then_some(files)
    }

    fn dispatch_drag_update(&mut self) {
        let (Some(position), Some(data)) = (self.xdnd.position.clone(), self.xdnd.data.clone())
        else {
            return;
        };
        let event = if self.xdnd.entered {
            MouseEvent::DragMoved {
                position: position.position,
                modifiers: position.modifiers,
                data,
            }
        } else {
            self.xdnd.entered = true;
            MouseEvent::DragEntered {
                position: position.position,
                modifiers: position.modifiers,
                data,
            }
        };
        self.xdnd.effect = accepted_effect(self.handle_event(Event::Mouse(event)));
    }

    fn handle_xdnd_leave(&mut self, source: XWindow) {
        if self.xdnd.source != Some(source) {
            return;
        }
        if self.xdnd.entered {
            self.handle_event(Event::Mouse(MouseEvent::DragLeft));
        }
        self.xdnd.reset();
    }

    fn handle_xdnd_drop(&mut self, data: [u32; 5]) {
        let source = data[0];
        if self.xdnd.source != Some(source) || !self.xdnd.supports_uri_list {
            self.send_xdnd_finished(source, None);
            self.xdnd.reset();
            return;
        }

        if self.xdnd.data.is_some() && self.xdnd.position.is_some() {
            self.complete_xdnd_drop();
            return;
        }

        self.xdnd.pending_drop = true;
        let time = if self.xdnd.version >= 1 { data[2] } else { CURRENT_TIME };
        self.request_xdnd_selection(time);
        if self.xdnd.selection_failed {
            self.reject_or_finish_xdnd();
        }
    }

    fn complete_xdnd_drop(&mut self) {
        if !self.xdnd.entered {
            self.dispatch_drag_update();
        }
        let effect = match (self.xdnd.position.clone(), self.xdnd.data.clone()) {
            (Some(position), Some(data)) => {
                accepted_effect(self.handle_event(Event::Mouse(MouseEvent::DragDropped {
                    position: position.position,
                    modifiers: position.modifiers,
                    data,
                })))
            }
            _ => None,
        };
        if let Some(source) = self.xdnd.source {
            self.send_xdnd_finished(source, effect);
        }
        self.xdnd.reset();
    }

    fn reject_or_finish_xdnd(&mut self) {
        let Some(source) = self.xdnd.source else {
            return;
        };
        if self.xdnd.pending_drop {
            self.send_xdnd_finished(source, None);
            self.xdnd.reset();
        } else {
            self.send_xdnd_status(source, None);
        }
    }

    fn effect_for_action(&self, action: Atom) -> Option<DropEffect> {
        let atoms = &self.window.xcb_connection.atoms;
        if action == atoms.XdndActionCopy {
            Some(DropEffect::Copy)
        } else if action == atoms.XdndActionMove {
            Some(DropEffect::Move)
        } else if action == atoms.XdndActionLink {
            Some(DropEffect::Link)
        } else if action == atoms.XdndActionPrivate {
            Some(DropEffect::Scroll)
        } else {
            None
        }
    }

    fn action_for_effect(&self, effect: Option<DropEffect>) -> Atom {
        let atoms = &self.window.xcb_connection.atoms;
        match effect {
            Some(DropEffect::Copy) => atoms.XdndActionCopy,
            Some(DropEffect::Move) => atoms.XdndActionMove,
            Some(DropEffect::Link) => atoms.XdndActionLink,
            Some(DropEffect::Scroll) => atoms.XdndActionPrivate,
            None => 0,
        }
    }

    fn send_xdnd_status(&self, source: XWindow, effect: Option<DropEffect>) {
        let atoms = &self.window.xcb_connection.atoms;
        let event = ClientMessageEvent::new(
            32,
            source,
            atoms.XdndStatus,
            [
                self.window.window_id,
                status_flags(effect.is_some()),
                0,
                0,
                self.action_for_effect(effect),
            ],
        );
        let _ =
            self.window.xcb_connection.conn.send_event(false, source, EventMask::NO_EVENT, event);
        let _ = self.window.xcb_connection.conn.flush();
    }

    fn send_xdnd_finished(&self, source: XWindow, effect: Option<DropEffect>) {
        let atoms = &self.window.xcb_connection.atoms;
        let action = self.action_for_effect(effect);
        let event = ClientMessageEvent::new(
            32,
            source,
            atoms.XdndFinished,
            finished_payload(self.window.window_id, effect, action),
        );
        let _ =
            self.window.xcb_connection.conn.send_event(false, source, EventMask::NO_EVENT, event);
        let _ = self.window.xcb_connection.conn.flush();
    }

    fn handle_event(&mut self, event: Event) -> EventStatus {
        // The close timeout permanently revokes callbacks before detaching a
        // wedged thread. Acquiring this guard with a CAS closes the check/call
        // race: once revoked, no new handler invocation can begin. A callback
        // already blocked in the graphics driver may finish later, but its
        // guard cannot overwrite the terminal revoked state.
        let _callback = match &self.parent_handle {
            Some(parent) => match parent.begin_callback() {
                Some(callback) => Some(callback),
                None => return EventStatus::Ignored,
            },
            None => None,
        };
        self.handler.on_event(&mut crate::Window::new(Window { inner: &self.window }), event)
    }

    fn handle_close_requested(&mut self) {
        // FIXME: handler should decide whether window stays open or not
        self.handle_must_close();
    }

    fn handle_must_close(&mut self) {
        self.handle_event(Event::Window(WindowEvent::WillClose));

        self.event_loop_running = false;
    }
}

fn accepted_effect(status: EventStatus) -> Option<DropEffect> {
    match status {
        EventStatus::AcceptDrop(effect) => Some(effect),
        EventStatus::Captured | EventStatus::Ignored => None,
    }
}

fn mouse_id(id: u8) -> MouseButton {
    match id {
        1 => MouseButton::Left,
        2 => MouseButton::Middle,
        3 => MouseButton::Right,
        8 => MouseButton::Back,
        9 => MouseButton::Forward,
        id => MouseButton::Other(id),
    }
}

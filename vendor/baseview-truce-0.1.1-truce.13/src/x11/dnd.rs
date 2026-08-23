use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;

use keyboard_types::Modifiers;
use x11rb::protocol::xproto::{Atom, AtomEnum, SelectionNotifyEvent, Timestamp, Window};

use crate::{DropData, DropEffect, Point};

pub(super) const XDND_VERSION: u32 = 5;
pub(super) const MAX_TYPE_LIST_ATOMS: u32 = 4_096;
pub(super) const MAX_URI_LIST_BYTES: usize = 4 * 1024 * 1024;
const MAX_URI_LIST_FILES: usize = 64;
const MAX_URI_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct EnterFlags {
    pub version: u32,
    pub has_type_list: bool,
}

pub(super) fn parse_enter_flags(flags: u32) -> EnterFlags {
    EnterFlags { version: flags >> 24, has_type_list: flags & 1 != 0 }
}

/// XdndPosition packs two signed 16-bit root coordinates into one word.
pub(super) fn unpack_root_coordinates(packed: u32) -> (i16, i16) {
    ((packed >> 16) as u16 as i16, packed as u16 as i16)
}

pub(super) fn status_flags(accepted: bool) -> u32 {
    // Bit 1 asks the source to keep sending positions even when our acceptance
    // changes. An empty rectangle alone is not interpreted consistently by all
    // source implementations.
    u32::from(accepted) | 2
}

pub(super) fn finished_payload(
    target: Window, effect: Option<DropEffect>, action: Atom,
) -> [u32; 5] {
    [target, u32::from(effect.is_some()), action, 0, 0]
}

#[derive(Debug, Clone)]
pub(super) struct DragPosition {
    pub position: Point,
    pub modifiers: Modifiers,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct XdndSelectionRequest {
    pub source: Window,
    pub requestor: Window,
    pub selection: Atom,
    pub target: Atom,
    pub property: Atom,
    pub time: Timestamp,
}

impl XdndSelectionRequest {
    pub fn matches(self, active_source: Option<Window>, event: &SelectionNotifyEvent) -> bool {
        active_source == Some(self.source)
            && event.requestor == self.requestor
            && event.selection == self.selection
            && event.target == self.target
            && (event.property == self.property || event.property == u32::from(AtomEnum::NONE))
            && event.time == self.time
    }
}

#[derive(Debug, Default)]
pub(super) struct XdndState {
    pub source: Option<Window>,
    pub version: u32,
    pub supports_uri_list: bool,
    pub selected_target: Option<Atom>,
    pub selection_request: Option<XdndSelectionRequest>,
    pub selection_failed: bool,
    pub data: Option<DropData>,
    pub position: Option<DragPosition>,
    pub entered: bool,
    pub effect: Option<DropEffect>,
    pub pending_drop: bool,
}

impl XdndState {
    pub fn begin(
        &mut self, source: Window, version: u32, supports_uri_list: bool,
        selected_target: Option<Atom>,
    ) {
        *self = Self {
            source: Some(source),
            version: version.min(XDND_VERSION),
            supports_uri_list,
            selected_target,
            ..Self::default()
        };
    }

    pub fn take_matching_selection_request(
        &mut self, event: &SelectionNotifyEvent,
    ) -> Option<XdndSelectionRequest> {
        let request = self.selection_request?;
        if !request.matches(self.source, event) {
            return None;
        }
        self.selection_request.take()
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// File-list MIME types accepted from inbound XDND, matching Buffr/Matari.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FileListKind {
    UriList,
    GnomeCopiedFiles,
    PlainPaths,
}

/// Parse RFC 2483 `text/uri-list` data into local Unix file paths.
///
/// Bad, remote, non-file, and NUL-containing entries are ignored. This keeps a
/// malformed entry from invalidating other files while never interpreting a
/// remote authority as a local path. The selection size is capped by the X11
/// property reader before this function is called and checked again here for
/// callers outside that path.
pub(super) fn parse_uri_list(data: &[u8]) -> Vec<PathBuf> {
    if data.len() > MAX_URI_LIST_BYTES {
        return Vec::new();
    }

    data.split(|byte| *byte == b'\n')
        .take(MAX_URI_LIST_FILES)
        .filter_map(|line| {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            if line.is_empty() || line.starts_with(b"#") {
                return None;
            }
            parse_file_uri(line)
        })
        .collect()
}

/// Parse GNOME/Nautilus `x-special/gnome-copied-files` (`copy|cut|link` + URIs).
pub(super) fn parse_gnome_copied_files(data: &[u8]) -> Vec<PathBuf> {
    if data.len() > MAX_URI_LIST_BYTES {
        return Vec::new();
    }
    let mut lines = data.split(|byte| *byte == b'\n');
    let Some(action) = lines.next() else {
        return Vec::new();
    };
    let action = action.strip_suffix(b"\r").unwrap_or(action);
    if !action.eq_ignore_ascii_case(b"copy")
        && !action.eq_ignore_ascii_case(b"cut")
        && !action.eq_ignore_ascii_case(b"link")
    {
        return parse_uri_list(data);
    }
    let remainder = lines.collect::<Vec<_>>();
    let mut payload = Vec::new();
    for line in remainder {
        payload.extend_from_slice(line);
        payload.push(b'\n');
    }
    parse_uri_list(&payload)
}

/// Parse a newline-delimited list of absolute local paths (no remote URIs).
pub(super) fn parse_plain_path_list(data: &[u8]) -> Vec<PathBuf> {
    if data.len() > MAX_URI_LIST_BYTES {
        return Vec::new();
    }
    if data.windows(5).any(|window| window.eq_ignore_ascii_case(b"file:")) {
        return parse_uri_list(data);
    }
    data.split(|byte| *byte == b'\n')
        .take(MAX_URI_LIST_FILES)
        .filter_map(|line| {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            if line.is_empty() || line.starts_with(b"#") || !line.starts_with(b"/") {
                return None;
            }
            if line.contains(&0) || line.contains(&b'?') || line.contains(&b'#') {
                return None;
            }
            Some(PathBuf::from(OsString::from_vec(line.to_vec())))
        })
        .collect()
}

pub(super) fn parse_file_list(kind: FileListKind, data: &[u8]) -> Vec<PathBuf> {
    match kind {
        FileListKind::UriList => parse_uri_list(data),
        FileListKind::GnomeCopiedFiles => parse_gnome_copied_files(data),
        FileListKind::PlainPaths => parse_plain_path_list(data),
    }
}

fn parse_file_uri(uri: &[u8]) -> Option<PathBuf> {
    if uri.len() > MAX_URI_BYTES {
        return None;
    }
    let scheme = uri.get(..5)?;
    if !scheme.eq_ignore_ascii_case(b"file:") {
        return None;
    }

    let remainder = &uri[5..];
    let path = if let Some(authority_and_path) = remainder.strip_prefix(b"//") {
        let slash = authority_and_path.iter().position(|byte| *byte == b'/')?;
        let authority = percent_decode(&authority_and_path[..slash])?;
        if !authority.is_empty() && !authority.eq_ignore_ascii_case(b"localhost") {
            return None;
        }
        &authority_and_path[slash..]
    } else {
        remainder
    };

    // File URIs used for XDND identify absolute local paths. A raw query or
    // fragment delimiter is URI syntax, not part of a filename (literal ones
    // must be percent encoded).
    if !path.starts_with(b"/") || path.contains(&b'?') || path.contains(&b'#') {
        return None;
    }

    let decoded = percent_decode(path)?;
    if decoded.contains(&0) {
        return None;
    }

    Some(PathBuf::from(OsString::from_vec(decoded)))
}

fn percent_decode(input: &[u8]) -> Option<Vec<u8>> {
    let mut output = Vec::with_capacity(input.len());
    let mut index = 0;
    while index < input.len() {
        if input[index] != b'%' {
            output.push(input[index]);
            index += 1;
            continue;
        }

        let high = hex(input.get(index + 1).copied()?)?;
        let low = hex(input.get(index + 2).copied()?)?;
        output.push(high << 4 | low);
        index += 3;
    }
    Some(output)
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStrExt;
    use x11rb::CURRENT_TIME;

    #[test]
    fn uri_list_parses_comments_crlf_spaces_and_non_utf8_paths() {
        let paths = parse_uri_list(
            b"# generated by a file manager\r\nfile:///tmp/one%20two.wav\r\nfile://localhost/tmp/%FF.raw\n",
        );

        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0], PathBuf::from("/tmp/one two.wav"));
        assert_eq!(paths[1].as_os_str().as_bytes(), b"/tmp/\xff.raw");
    }

    #[test]
    fn uri_list_ignores_remote_unsupported_and_malformed_entries() {
        let paths = parse_uri_list(
            b"https://example.com/a.wav\nfile://server/share/a.wav\nfile:///tmp/bad%2.wav\nfile:///tmp/nul%00.wav\nfile:relative.wav\nfile:///tmp/good.wav\n",
        );

        assert_eq!(paths, vec![PathBuf::from("/tmp/good.wav")]);
    }

    #[test]
    fn uri_list_requires_encoded_query_and_fragment_characters() {
        let paths = parse_uri_list(
            b"file:///tmp/query?.wav\nfile:///tmp/fragment#.wav\nfile:///tmp/query%3F.wav\nfile:///tmp/fragment%23.wav\n",
        );

        assert_eq!(
            paths,
            vec![PathBuf::from("/tmp/query?.wav"), PathBuf::from("/tmp/fragment#.wav")]
        );
    }

    #[test]
    fn enter_flags_extract_version_and_extended_type_flag() {
        assert_eq!(
            parse_enter_flags((5 << 24) | 1),
            EnterFlags { version: 5, has_type_list: true }
        );
        assert_eq!(parse_enter_flags(3 << 24), EnterFlags { version: 3, has_type_list: false });
    }

    #[test]
    fn packed_root_coordinates_preserve_negative_values() {
        let packed = ((-12_i16 as u16 as u32) << 16) | (-34_i16 as u16 as u32);
        assert_eq!(unpack_root_coordinates(packed), (-12, -34));
    }

    #[test]
    fn status_flags_accept_and_request_future_positions() {
        assert_eq!(status_flags(false), 2);
        assert_eq!(status_flags(true), 3);
    }

    #[test]
    fn finished_payload_reports_the_selected_action() {
        assert_eq!(finished_payload(42, Some(DropEffect::Copy), 99), [42, 1, 99, 0, 0]);
        assert_eq!(finished_payload(42, None, 0), [42, 0, 0, 0, 0]);
    }

    #[test]
    fn uri_list_caps_path_count_and_individual_uri_size() {
        let many = (0..100).map(|index| format!("file:///tmp/{index}\n")).collect::<String>();
        assert_eq!(parse_uri_list(many.as_bytes()).len(), MAX_URI_LIST_FILES);

        let oversized = format!("file:///{}", "a".repeat(MAX_URI_BYTES));
        assert!(parse_uri_list(oversized.as_bytes()).is_empty());
    }

    const SOURCE: Window = 11;
    const REQUESTOR: Window = 22;
    const SELECTION: Atom = 33;
    const TARGET: Atom = 44;
    const PROPERTY: Atom = 55;
    const TIME: u32 = 66;

    fn selection_request(time: u32) -> XdndSelectionRequest {
        XdndSelectionRequest {
            source: SOURCE,
            requestor: REQUESTOR,
            selection: SELECTION,
            target: TARGET,
            property: PROPERTY,
            time,
        }
    }

    fn selection_notify(time: u32, property: Atom) -> SelectionNotifyEvent {
        SelectionNotifyEvent {
            time,
            requestor: REQUESTOR,
            selection: SELECTION,
            target: TARGET,
            property,
            ..SelectionNotifyEvent::default()
        }
    }

    #[test]
    fn selection_completion_matches_the_exact_outstanding_request() {
        let request = selection_request(TIME);
        let event = selection_notify(TIME, PROPERTY);

        assert!(request.matches(Some(SOURCE), &event));
    }

    #[test]
    fn selection_completion_accepts_none_property_as_conversion_failure() {
        let request = selection_request(TIME);
        let event = selection_notify(TIME, u32::from(AtomEnum::NONE));

        assert!(request.matches(Some(SOURCE), &event));
    }

    #[test]
    fn selection_completion_rejects_every_unrelated_protocol_field() {
        let request = selection_request(TIME);
        let exact = selection_notify(TIME, PROPERTY);

        assert!(!request.matches(Some(SOURCE + 1), &exact));

        let mismatches = [
            SelectionNotifyEvent { requestor: REQUESTOR + 1, ..exact },
            SelectionNotifyEvent { selection: SELECTION + 1, ..exact },
            SelectionNotifyEvent { target: TARGET + 1, ..exact },
            SelectionNotifyEvent { property: PROPERTY + 1, ..exact },
            SelectionNotifyEvent { time: TIME + 1, ..exact },
        ];
        for event in mismatches {
            assert!(!request.matches(Some(SOURCE), &event));
        }
    }

    #[test]
    fn current_time_completion_still_requires_exact_time() {
        let request = selection_request(CURRENT_TIME);

        assert!(request.matches(Some(SOURCE), &selection_notify(CURRENT_TIME, PROPERTY)));
        assert!(!request.matches(Some(SOURCE), &selection_notify(TIME, PROPERTY)));
    }

    #[test]
    fn unrelated_completion_does_not_consume_the_outstanding_request() {
        let request = selection_request(TIME);
        let mut state = XdndState {
            source: Some(SOURCE),
            selection_request: Some(request),
            ..XdndState::default()
        };
        let stale = selection_notify(TIME + 1, PROPERTY);

        assert!(state.take_matching_selection_request(&stale).is_none());
        assert_eq!(state.selection_request, Some(request));
        assert_eq!(
            state.take_matching_selection_request(&selection_notify(TIME, PROPERTY)),
            Some(request)
        );
        assert!(state.selection_request.is_none());
    }

    #[test]
    fn beginning_a_new_drag_invalidates_the_outstanding_selection_request() {
        let mut state = XdndState::default();
        state.begin(SOURCE, XDND_VERSION, true, Some(TARGET));
        state.selection_request = Some(selection_request(TIME));

        state.begin(SOURCE + 1, XDND_VERSION, true, Some(TARGET));

        assert!(state.selection_request.is_none());
    }

    #[test]
    fn gnome_copied_files_skips_the_action_line() {
        let paths = parse_gnome_copied_files(b"copy\nfile:///tmp/pad.flac\nfile:///tmp/loop.wav\n");
        assert_eq!(paths, vec![PathBuf::from("/tmp/pad.flac"), PathBuf::from("/tmp/loop.wav")]);
    }

    #[test]
    fn gnome_copied_files_falls_back_to_uri_list_without_an_action() {
        let paths = parse_gnome_copied_files(b"file:///tmp/direct.aiff\n");
        assert_eq!(paths, vec![PathBuf::from("/tmp/direct.aiff")]);
    }

    #[test]
    fn plain_path_list_accepts_absolute_local_paths() {
        let paths = parse_plain_path_list(b"/tmp/one.wav\n/tmp/two.flac\n");
        assert_eq!(paths, vec![PathBuf::from("/tmp/one.wav"), PathBuf::from("/tmp/two.flac")]);
    }

    #[test]
    fn plain_path_list_rejects_relative_and_remote_entries() {
        let paths =
            parse_plain_path_list(b"relative.wav\nhttps://example.com/a.wav\n/tmp/ok.wav\n");
        assert_eq!(paths, vec![PathBuf::from("/tmp/ok.wav")]);
    }
}

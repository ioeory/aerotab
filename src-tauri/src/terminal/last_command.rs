//! Passive shell integration observer. Never transforms transport bytes.

const OUTPUT_LIMIT: usize = 64 * 1024;
const OSC_LIMIT: usize = 8 * 1024;
const CSI_LIMIT: usize = 64;
const MARKER: &[u8] = b"\x1b]633;";
const TMUX_DCS: &[u8] = b"tmux;";
const INTERACTIVE_MODES: [u16; 13] = [
    9, 47, 1000, 1001, 1002, 1003, 1004, 1005, 1006, 1015, 1016, 1047, 1049,
];

/// xterm's onData mixes user input with automatic replies. Only exempt a single
/// complete, recognized reply from capture invalidation; never change its routing.
pub(crate) fn is_status_reply(bytes: &[u8]) -> bool {
    if bytes.len() > CSI_LIMIT {
        return false;
    }
    match bytes {
        b"\x1b[0n" | b"\x1b[?1;2c" | b"\x1b[?6c" | b"\x1b[>0;276;0c" | b"\x1b[>85;95;0c"
        | b"\x1b[>83;40003;0c" => true,
        [0x1b, b'[', body @ .., b'R'] => {
            let private = body.starts_with(b"?");
            let (row, col) = split_field(body.strip_prefix(b"?").unwrap_or(body));
            match (decimal(row), col.and_then(decimal)) {
                // xterm also emits CSI 1;2..16 R for modified F3; treat those
                // ambiguous non-private replies as input, not as status.
                (Some(row), Some(col)) if row > 0 && col > 0 => {
                    private || row != 1 || !(2..=16).contains(&col)
                }
                _ => false,
            }
        }
        [0x1b, b'[', body @ .., b'$', b'y'] => {
            let (mode, state) = split_field(body.strip_prefix(b"?").unwrap_or(body));
            decimal(mode).is_some() && matches!(state, Some([b'0'..=b'4']))
        }
        [0x1b, b'[', kind @ (b'4' | b'6' | b'8'), b';', size @ .., b't'] => {
            let (height, width) = split_field(size);
            match (decimal(height), width.and_then(decimal)) {
                (Some(height), Some(width)) => *kind != b'8' || (height > 0 && width > 0),
                _ => false,
            }
        }
        [0x1b, b']', body @ .., 0x1b, b'\\'] => {
            let (kind, rest) = split_field(body);
            let rgb = match (kind, rest) {
                (b"4", Some(rest)) => {
                    let (index, rgb) = split_field(rest);
                    if !matches!(decimal(index), Some(0..=255)) {
                        return false;
                    }
                    rgb
                }
                (b"10" | b"11" | b"12", rgb) => rgb,
                _ => None,
            };
            let Some(rgb) = rgb.and_then(|rgb| rgb.strip_prefix(b"rgb:")) else {
                return false;
            };
            let mut components = rgb.split(|&byte| byte == b'/');
            (0..3).all(|_| {
                components
                    .next()
                    .is_some_and(|part| part.len() == 4 && part.iter().all(u8::is_ascii_hexdigit))
            }) && components.next().is_none()
        }
        [0x1b, b'P', body @ .., 0x1b, b'\\'] => match body {
            b"0$r" | b"1$r0m" | b"1$r0\"q" | b"1$r1\"q" | b"1$r61;1\"p" => true,
            [b'1', b'$', b'r', b'1'..=b'6', b' ', b'q'] => true,
            [b'1', b'$', b'r', margins @ .., b'r'] => {
                let (top, bottom) = split_field(margins);
                matches!((decimal(top), bottom.and_then(decimal)), (Some(top), Some(bottom)) if top > 0 && top <= bottom)
            }
            _ => false,
        },
        _ => false,
    }
}

fn decimal(bytes: &[u8]) -> Option<u32> {
    if bytes.is_empty()
        || bytes.len() > 10
        || (bytes.len() > 1 && bytes.starts_with(b"0"))
        || !bytes.iter().all(u8::is_ascii_digit)
    {
        return None;
    }
    std::str::from_utf8(bytes).ok()?.parse().ok()
}

pub(crate) struct Completed {
    pub(crate) command: String,
    pub(crate) output: Vec<u8>,
    pub(crate) exit_code: Option<i32>,
    pub(crate) truncated: bool,
}

#[derive(Default)]
enum Phase {
    #[default]
    Idle,
    Armed(String),
    Running(Completed),
    Discarded,
}

#[derive(Default)]
enum Scan {
    #[default]
    Ground,
    Escape,
    Csi(Vec<u8>),
    Osc {
        bytes: Vec<u8>,
        escaped: bool,
    },
    String {
        osc: bool,
        escaped: bool,
        tmux_prefix: Option<usize>,
    },
}

#[derive(Default)]
pub(crate) struct Tracker {
    scan: Scan,
    phase: Phase,
    interactive_modes: u16,
    known_tmux: bool,
    pub(crate) last: Option<Completed>,
}

impl Tracker {
    pub(crate) fn observe(&mut self, mut bytes: &[u8]) {
        while let Some((&byte, rest)) = bytes.split_first() {
            if matches!(self.scan, Scan::Ground) {
                let count = bytes.iter().position(|&b| b == 0x1b).unwrap_or(bytes.len());
                if count != 0 {
                    self.append(&bytes[..count]);
                    bytes = &bytes[count..];
                    continue;
                }
            }
            self.scan = match std::mem::take(&mut self.scan) {
                Scan::Ground => Scan::Escape,
                Scan::Escape => match byte {
                    b']' => Scan::Osc {
                        bytes: b"\x1b]".to_vec(),
                        escaped: false,
                    },
                    b'[' => {
                        self.append(b"\x1b[");
                        Scan::Csi(Vec::new())
                    }
                    b'P' | b'_' | b'^' | b'X' => {
                        self.append(&[0x1b, byte]);
                        Scan::String {
                            osc: false,
                            escaped: false,
                            tmux_prefix: (byte == b'P').then_some(0),
                        }
                    }
                    b'c' => {
                        self.interactive_modes = 0;
                        self.reset();
                        Scan::Ground
                    }
                    0x1b => {
                        self.append(b"\x1b");
                        Scan::Escape
                    }
                    _ => {
                        self.append(&[0x1b, byte]);
                        Scan::Ground
                    }
                },
                Scan::Csi(mut body) => {
                    if byte == 0x1b {
                        self.discard();
                        Scan::Escape
                    } else {
                        self.append(&[byte]);
                        match byte {
                            0x40..=0x7e => {
                                self.csi(&body, byte);
                                Scan::Ground
                            }
                            0x20..=0x3f if body.len() <= CSI_LIMIT => {
                                body.push(byte);
                                if body.len() > CSI_LIMIT {
                                    self.discard();
                                }
                                Scan::Csi(body)
                            }
                            0x20..=0x3f => Scan::Csi(body),
                            0x00..=0x17 | 0x19 | 0x1c..=0x1f | 0x7f => Scan::Csi(body),
                            _ => {
                                self.discard();
                                Scan::Ground
                            }
                        }
                    }
                }
                Scan::Osc { mut bytes, escaped } => {
                    let marker = marker_prefix(&bytes);
                    let terminated = matches!(byte, 0x07) || (escaped && byte == b'\\');
                    if byte == 0x18 || byte == 0x1a {
                        if marker {
                            self.discard();
                        } else {
                            self.append(&bytes);
                            self.append(&[byte]);
                        }
                        Scan::Ground
                    } else if terminated {
                        let end = bytes.len() - usize::from(escaped && byte == b'\\');
                        let body = bytes[..end].strip_prefix(MARKER).filter(|body| {
                            matches!(split_field(body).0, b"A" | b"B" | b"C" | b"D" | b"E")
                        });
                        if let Some(body) = body {
                            self.marker(body);
                        } else {
                            self.append(&bytes);
                            self.append(&[byte]);
                        }
                        Scan::Ground
                    } else if (escaped || bytes.len() >= OSC_LIMIT) && marker {
                        self.discard();
                        Scan::String {
                            osc: true,
                            escaped: byte == 0x1b && !escaped,
                            tmux_prefix: None,
                        }
                    } else {
                        bytes.push(byte);
                        if MARKER.starts_with(&bytes) || marker_prefix(&bytes) {
                            Scan::Osc {
                                bytes,
                                escaped: byte == 0x1b,
                            }
                        } else {
                            self.append(&bytes);
                            Scan::String {
                                osc: true,
                                escaped: byte == 0x1b,
                                tmux_prefix: None,
                            }
                        }
                    }
                }
                Scan::String {
                    osc,
                    escaped,
                    mut tmux_prefix,
                } => {
                    self.append(&[byte]);
                    if let Some(index) = tmux_prefix {
                        if TMUX_DCS[index] != byte {
                            tmux_prefix = None;
                        } else if index + 1 == TMUX_DCS.len() {
                            self.known_tmux = true;
                            self.discard();
                            tmux_prefix = None;
                        } else {
                            tmux_prefix = Some(index + 1);
                        }
                    }
                    if (osc && byte == 0x07)
                        || (escaped && byte == b'\\')
                        || matches!(byte, 0x18 | 0x1a)
                    {
                        Scan::Ground
                    } else {
                        Scan::String {
                            osc,
                            escaped: byte == 0x1b && !escaped,
                            tmux_prefix,
                        }
                    }
                }
            };
            bytes = rest;
        }
    }

    fn append(&mut self, bytes: &[u8]) {
        if let Phase::Running(result) = &mut self.phase {
            let keep = bytes.len().min(OUTPUT_LIMIT - result.output.len());
            result.output.extend_from_slice(&bytes[..keep]);
            result.truncated |= keep < bytes.len();
        }
    }

    pub(crate) fn note_input(&mut self) {
        self.last = None;
        if matches!(self.phase, Phase::Armed(_) | Phase::Running(_)) {
            self.discard();
        }
    }

    fn discard(&mut self) {
        self.phase = Phase::Discarded;
        self.last = None;
    }

    fn reset(&mut self) {
        self.phase = Phase::Idle;
        self.last = None;
    }

    fn csi(&mut self, body: &[u8], final_byte: u8) {
        if body == b"!" && final_byte == b'p' {
            self.reset();
            return;
        }
        let Some(modes) = body.strip_prefix(b"?") else {
            return;
        };
        if !matches!(final_byte, b'h' | b'l') {
            return;
        }
        for mode in modes.split(|&b| b == b';') {
            let index = std::str::from_utf8(mode)
                .ok()
                .and_then(|s| s.parse::<u16>().ok())
                .and_then(|mode| INTERACTIVE_MODES.iter().position(|&known| mode == known));
            if let Some(index) = index {
                if final_byte == b'h' {
                    self.interactive_modes |= 1 << index;
                    self.discard();
                } else {
                    self.interactive_modes &= !(1 << index);
                }
            }
        }
    }

    fn marker(&mut self, body: &[u8]) {
        if self.known_tmux || self.interactive_modes != 0 {
            self.discard();
            return;
        }
        let (kind, fields) = split_field(body);
        if matches!(kind, b"E" | b"C") {
            // Never present a previous result as the command that just ran.
            self.last = None;
        }
        let old = std::mem::take(&mut self.phase);
        self.phase = match (kind, fields, old) {
            (b"E", Some(fields), Phase::Idle) => {
                // The optional VS Code-style nonce is metadata, not authentication.
                match decode_command_fields(fields) {
                    Some(command) if known_tmux_command(&command) => {
                        self.known_tmux = true;
                        Phase::Discarded
                    }
                    Some(command) => Phase::Armed(command),
                    None => Phase::Discarded,
                }
            }
            (b"C", None, Phase::Armed(command)) => Phase::Running(Completed {
                command,
                output: Vec::new(),
                exit_code: None,
                truncated: false,
            }),
            (b"D", fields, old) => {
                if let Phase::Running(mut result) = old {
                    let code = match fields {
                        None => Some(None),
                        Some(field) => std::str::from_utf8(field)
                            .ok()
                            .and_then(|s| s.parse::<i32>().ok())
                            .map(Some),
                    };
                    if let Some(code) = code {
                        result.exit_code = code;
                        if result.truncated {
                            trim_incomplete_utf8_tail(&mut result.output);
                        }
                        self.last = Some(result);
                    }
                }
                Phase::Idle
            }
            (b"A" | b"B", None, _) => Phase::Idle,
            (b"E" | b"C" | b"A" | b"B", _, _) => Phase::Discarded,
            (_, _, old) => old,
        };
    }
}

fn marker_prefix(bytes: &[u8]) -> bool {
    matches!(
        bytes.strip_prefix(MARKER),
        Some([b'A'..=b'E']) | Some([b'A'..=b'E', b';', ..]) | Some([b'A'..=b'E', 0x1b])
    )
}

fn trim_incomplete_utf8_tail(bytes: &mut Vec<u8>) {
    // At most three trailing bytes can be the beginning of an incomplete
    // UTF-8 scalar. Don't remove earlier invalid terminal bytes.
    let start = bytes.len().saturating_sub(3);
    if let Some(offset) = bytes[start..].iter().rposition(|&byte| byte & 0xc0 != 0x80) {
        let index = start + offset;
        if std::str::from_utf8(&bytes[index..]).is_err_and(|err| err.error_len().is_none()) {
            bytes.truncate(index);
        }
    }
}

fn split_field(bytes: &[u8]) -> (&[u8], Option<&[u8]>) {
    bytes
        .iter()
        .position(|&b| b == b';')
        .map_or((bytes, None), |index| {
            (&bytes[..index], Some(&bytes[index + 1..]))
        })
}

fn decode_command_fields(fields: &[u8]) -> Option<String> {
    let (mut encoded, nonce) = split_field(fields);
    if nonce.is_some_and(|value| value.contains(&b';')) {
        return None;
    }
    let mut decoded = Vec::with_capacity(encoded.len());
    while let Some((&byte, rest)) = encoded.split_first() {
        let (byte, rest) = if byte == b'\\' {
            match rest {
                [b'\\', rest @ ..] => (b'\\', rest),
                [b'x', hi, lo, rest @ ..] => (
                    ((char::from(*hi).to_digit(16)? << 4) | char::from(*lo).to_digit(16)?) as u8,
                    rest,
                ),
                _ => return None,
            }
        } else {
            (byte, rest)
        };
        if matches!(byte, 0x00..=0x08 | 0x0b..=0x1f | 0x7f) {
            return None;
        }
        decoded.push(byte);
        encoded = rest;
    }
    let decoded = String::from_utf8(decoded).ok()?;
    (!decoded.trim().is_empty()).then_some(decoded)
}

fn known_tmux_command(command: &str) -> bool {
    for token in command.split_ascii_whitespace() {
        let token = token.trim_matches(['\'', '"']);
        let program = token.rsplit('/').next().unwrap_or(token);
        match program {
            "tmux" | "tmux.exe" => return true,
            "env" | "exec" | "command" | "sudo" | "nohup" => (),
            _ if token.contains('=') || token.starts_with('-') => (),
            _ => return false,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const START: &[u8] = b"\x1b]633;E;printf hello\x07\x1b]633;C\x1b\\";
    const DONE: &[u8] = b"\x1b]633;D;7\x07";

    #[test]
    fn status_replies_require_the_entire_payload_and_exact_shape() {
        for reply in [
            &b"\x1b[4;0;0t"[..],
            b"\x1b[20;2$y",
            b"\x1b]4;255;rgb:ffff/1234/abcd\x1b\\",
            b"\x1bP1$r1;24r\x1b\\",
        ] {
            assert!(is_status_reply(reply), "valid reply {reply:?}");
        }
        for input in [
            &b"\x1b[6n"[..],
            b"\x1b[1;16R",
            b"\x1b[?1;2;3R",
            b"\x1b[?25;5$y",
            b"\x1b[8;0;80t",
            b"\x1b]4;256;rgb:ffff/1234/abcd\x1b\\",
            b"\x1b]11;rgb:0000/0000/0000\x1b\\x",
            b"\x1bP1$r2;1r\x1b\\",
        ] {
            assert!(!is_status_reply(input), "unrecognized input {input:?}");
        }
    }

    #[test]
    fn fragmented_markers_and_output_capture_only_command_range() {
        let mut tracker = Tracker::default();
        for byte in b"prompt\x1b]633;E;printf \\x3b\\\\;nonce\x07\x1b]633;C\x1b\\\x1b[31m\xe4\xbd\xa0\r\n\x1b]633;D;7\x1b\\prompt" {
            tracker.observe(&[*byte]);
        }
        let result = tracker.last.unwrap();
        assert_eq!(result.command, "printf ;\\");
        assert_eq!(result.output, b"\x1b[31m\xe4\xbd\xa0\r\n");
        assert_eq!(result.exit_code, Some(7));
        assert!(!result.truncated);
    }

    #[test]
    fn caps_output_without_counting_the_end_marker() {
        let mut tracker = Tracker::default();
        tracker.observe(START);
        tracker.observe(&vec![b'x'; OUTPUT_LIMIT]);
        tracker.observe(DONE);
        assert_eq!(tracker.last.as_ref().unwrap().output.len(), OUTPUT_LIMIT);
        assert!(!tracker.last.as_ref().unwrap().truncated);
        tracker.observe(START);
        tracker.observe(&vec![b'x'; OUTPUT_LIMIT + 1]);
        tracker.observe(DONE);
        assert_eq!(tracker.last.as_ref().unwrap().output.len(), OUTPUT_LIMIT);
        assert!(tracker.last.as_ref().unwrap().truncated);
    }

    #[test]
    fn captures_similar_osc_prefixes_without_stripping_them() {
        let mut tracker = Tracker::default();
        tracker.observe(START);
        let output = b"\x1b]63\x07\x1b]6330;D;0\x1b\\\x1b]633;Debug\x07\x1b]633;D\x1b\x07\x1b]633;P;Cwd=/tmp\x07";
        for byte in output {
            tracker.observe(&[*byte]);
        }
        tracker.observe(DONE);
        assert_eq!(tracker.last.unwrap().output, output);
    }

    #[test]
    fn long_non_marker_osc_633_payload_remains_opaque_output() {
        let mut tracker = Tracker::default();
        tracker.observe(START);
        let mut output = b"\x1b]633;P;Cwd=".to_vec();
        output.extend(vec![b'x'; OSC_LIMIT + 1]);
        output.push(0x07);
        tracker.observe(&output);
        tracker.observe(DONE);
        assert_eq!(
            tracker.last.map(|result| result.output.len()),
            Some(output.len())
        );
    }

    #[test]
    fn truncation_does_not_keep_a_partial_utf8_character() {
        let mut tracker = Tracker::default();
        tracker.observe(START);
        tracker.observe(&vec![b'x'; OUTPUT_LIMIT - 1]);
        tracker.observe("你".as_bytes());
        tracker.observe(DONE);
        let result = tracker.last.unwrap();
        assert!(result.truncated);
        assert_eq!(result.output.len(), OUTPUT_LIMIT - 1);
        assert!(result.output.iter().all(|&byte| byte == b'x'));
    }

    #[test]
    fn missing_duplicate_or_out_of_order_markers_never_complete() {
        for sequence in [
            &b"\x1b]633;C\x07\x1b]633;D;0\x07"[..],
            b"\x1b]633;E;echo\x07\x1b]633;D;0\x07",
            b"\x1b]633;E;echo\x07\x1b]633;E;echo again\x07\x1b]633;C\x07\x1b]633;D;0\x07",
            b"\x1b]633;E;echo\x07\x1b]633;C\x07\x1b]633;C\x07\x1b]633;D;0\x07",
            b"\x1b]633;E;echo\x07\x1b]633;C\x07\x1b]633;D;bad\x07\x1b]633;D;0\x07",
            b"\x1b]633;E;echo\x07\x1b]633;C\x07\x1b]633;A\x07\x1b]633;D;0\x07",
        ] {
            let mut tracker = Tracker::default();
            tracker.observe(sequence);
            assert!(tracker.last.is_none());
            tracker.observe(START);
            tracker.observe(b"\x1b]633;D\x07");
            assert_eq!(tracker.last.unwrap().exit_code, None);
        }
    }

    #[test]
    fn interactive_modes_input_and_known_tmux_discard_capture() {
        for interactive in [
            &b"\x1b[?1049h\x1b[?1049l"[..],
            b"\x1b[?25;1000h\x1b[?1000l",
            b"\x1bPtmux;\x1b\x1b]633;D;0\x07\x1b\\",
        ] {
            let mut tracker = Tracker::default();
            tracker.observe(START);
            for byte in interactive {
                tracker.observe(&[*byte]);
            }
            tracker.observe(DONE);
            assert!(tracker.last.is_none());
        }
        let mut tracker = Tracker::default();
        tracker.observe(START);
        tracker.note_input();
        tracker.observe(DONE);
        assert!(tracker.last.is_none());
        tracker.observe(b"\x1b]633;E;exec /usr/bin/tmux attach\x07\x1b]633;C\x07");
        tracker.observe(START); // an inner shell cannot re-arm before a boundary
        tracker.observe(DONE);
        assert!(tracker.last.is_none());
    }

    #[test]
    fn resets_discard_completed_and_in_flight_data() {
        let mut tracker = Tracker::default();
        tracker.observe(START);
        tracker.observe(DONE);
        tracker.observe(b"\x1bc");
        assert!(tracker.last.is_none());
        tracker.observe(START);
        tracker.observe(b"\x1b[!p");
        tracker.observe(DONE);
        assert!(tracker.last.is_none());
        tracker.observe(b"\x1b[?1049h\x1b[!p");
        tracker.observe(START);
        tracker.observe(DONE);
        assert!(
            tracker.last.is_none(),
            "DECSTR does not leave alternate screen"
        );
    }

    #[test]
    fn input_at_prompt_invalidates_old_result_without_blocking_next_command() {
        let mut tracker = Tracker::default();
        tracker.observe(START);
        tracker.observe(DONE);
        tracker.note_input();
        assert!(tracker.last.is_none());
        tracker.observe(START);
        tracker.observe(DONE);
        assert!(tracker.last.is_some());
    }

    #[test]
    fn oversized_metadata_is_dropped_and_opaque_strings_are_not_parsed() {
        let mut tracker = Tracker::default();
        tracker.observe(b"\x1b]633;E;");
        tracker.observe(&vec![b'x'; OSC_LIMIT + 1]);
        tracker.observe(b"\x07\x1b]633;C\x07");
        tracker.observe(DONE);
        assert!(tracker.last.is_none());
        tracker.observe(START);
        tracker.observe(b"\x1bPignore \x1b]633;D;0\x07\x1b\\");
        assert!(tracker.last.is_none());
        tracker.observe(DONE);
        assert_eq!(
            tracker.last.unwrap().output,
            b"\x1bPignore \x1b]633;D;0\x07\x1b\\"
        );
    }
}

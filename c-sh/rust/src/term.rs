//! The terminal state behind a session: a vt100 parser that renders the current screen (what a
//! person would see), a capped transcript of raw output with an absolute offset (so an agent can
//! read only what is new), and the mapping from key names to the bytes a terminal sends.

use crate::{err, Result};

/// How much raw output to keep for `read`/`logs` (older bytes drop off the front).
const TRANSCRIPT_CAP: usize = 1 << 20;

/// What the parser tells us beyond the screen: the window title, and the replies a terminal owes
/// the program (cursor position and device attribute reports), which the session sends back.
#[derive(Default)]
pub struct Events {
    title: String,
    replies: Vec<u8>,
}

impl vt100::Callbacks for Events {
    fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
        self.title = String::from_utf8_lossy(title).into_owned();
    }

    fn unhandled_csi(&mut self, screen: &mut vt100::Screen, i1: Option<u8>, _i2: Option<u8>, params: &[&[u16]], c: char) {
        let first = params.first().and_then(|p| p.first()).copied().unwrap_or(0);
        match (i1, c, first) {
            // DSR 6: where is the cursor? (1-based row;col)
            (None, 'n', 6) => {
                let (row, col) = screen.cursor_position();
                self.replies.extend_from_slice(format!("\x1b[{};{}R", row + 1, col + 1).as_bytes());
            }
            // DSR 5: status — fine.
            (None, 'n', 5) => self.replies.extend_from_slice(b"\x1b[0n"),
            // DA1: a VT220 with ANSI colour, as xterm answers.
            (None, 'c', 0) => self.replies.extend_from_slice(b"\x1b[?62;22c"),
            // DA2 (ESC [ > c)
            (Some(b'>'), 'c', 0) => self.replies.extend_from_slice(b"\x1b[>0;0;0c"),
            _ => {}
        }
    }
}

pub struct Terminal {
    parser: vt100::Parser<Events>,
    cols: u16,
    rows: u16,
    /// Recent raw output; `start` is the absolute offset of its first byte.
    transcript: Vec<u8>,
    start: usize,
    total: usize,
}

impl Terminal {
    pub fn new(cols: u16, rows: u16) -> Terminal {
        Terminal { parser: vt100::Parser::new_with_callbacks(rows, cols, 0, Events::default()), cols, rows, transcript: Vec::new(), start: 0, total: 0 }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
        self.transcript.extend_from_slice(bytes);
        self.total += bytes.len();
        if self.transcript.len() > TRANSCRIPT_CAP {
            let drop = self.transcript.len() - TRANSCRIPT_CAP;
            self.transcript.drain(..drop);
            self.start += drop;
        }
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.cols = cols;
        self.rows = rows;
        self.parser.screen_mut().set_size(rows, cols);
    }

    pub fn size(&self) -> (u16, u16) {
        (self.cols, self.rows)
    }

    /// Replies the program asked for (answers to terminal queries), to send back as input.
    pub fn take_replies(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.parser.callbacks_mut().replies)
    }

    /// The total number of output bytes seen (the end offset for `read --since`).
    pub fn total(&self) -> usize {
        self.total
    }

    /// The rendered screen as text (trailing blanks trimmed), what is on it right now.
    pub fn screen(&self) -> String {
        self.parser.screen().contents()
    }

    /// The cursor's row and column (0-based).
    pub fn cursor(&self) -> (u16, u16) {
        self.parser.screen().cursor_position()
    }

    /// The title the program set with an OSC sequence, if any.
    pub fn title(&self) -> String {
        self.parser.callbacks().title.clone()
    }

    /// Raw output from absolute offset `since` to the end, and the new end offset. Output that has
    /// already dropped off the front is reported as lost (the returned offset starts at `start`).
    pub fn since(&self, since: usize) -> (Vec<u8>, usize, bool) {
        let from = since.max(self.start);
        let lost = since < self.start;
        let slice = self.transcript[from - self.start..].to_vec();
        (slice, self.total, lost)
    }

    /// The whole kept transcript (for `logs`).
    pub fn transcript(&self) -> &[u8] {
        &self.transcript
    }
}

/// Terminal output as plain text, the way it reads on paper: escape sequences (CSI, OSC, DCS
/// and the rest) taken out, a carriage return overwriting the line from its start (progress
/// bars keep their last state), backspace stepping back, "erase line" honoured, CRLF as LF.
pub fn plain(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut out = String::with_capacity(text.len());
    let mut line: Vec<char> = Vec::new();
    let mut col = 0usize;
    let mut chars = text.chars().peekable();
    let put = |line: &mut Vec<char>, col: &mut usize, c: char| {
        if *col < line.len() {
            line[*col] = c;
        } else {
            while line.len() < *col {
                line.push(' ');
            }
            line.push(c);
        }
        *col += 1;
    };
    while let Some(c) = chars.next() {
        match c {
            '\n' => {
                out.extend(line.iter().collect::<String>().trim_end().chars());
                out.push('\n');
                line.clear();
                col = 0;
            }
            '\r' => col = 0,
            '\u{8}' => col = col.saturating_sub(1),
            '\t' => {
                let next = (col / 8 + 1) * 8;
                while col < next {
                    put(&mut line, &mut col, ' ');
                }
            }
            '\u{1b}' => match chars.next() {
                // CSI: parameters, intermediates, one final byte.
                Some('[') => {
                    let mut params = String::new();
                    let mut fin = '\0';
                    for d in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&d) {
                            fin = d;
                            break;
                        }
                        params.push(d);
                    }
                    if fin == 'K' {
                        match params.as_str() {
                            "" | "0" => line.truncate(col),
                            "1" => {
                                for x in line.iter_mut().take(col + 1) {
                                    *x = ' ';
                                }
                            }
                            _ => line.clear(),
                        }
                    } else if fin == 'C' {
                        col += params.parse::<usize>().unwrap_or(1).max(1);
                    } else if fin == 'D' {
                        col = col.saturating_sub(params.parse::<usize>().unwrap_or(1).max(1));
                    } else if fin == 'G' {
                        col = params.parse::<usize>().unwrap_or(1).max(1) - 1;
                    }
                }
                // OSC, DCS, SOS, PM, APC: a string up to BEL or ST (ESC \).
                Some(']') | Some('P') | Some('X') | Some('^') | Some('_') => {
                    while let Some(d) = chars.next() {
                        if d == '\u{7}' {
                            break;
                        }
                        if d == '\u{1b}' {
                            if chars.peek() == Some(&'\\') {
                                chars.next();
                            }
                            break;
                        }
                    }
                }
                // ESC with intermediates (charset selection ESC ( B, ...), then a final byte.
                Some(d) if ('\u{20}'..='\u{2f}').contains(&d) => {
                    for e in chars.by_ref() {
                        if !('\u{20}'..='\u{2f}').contains(&e) {
                            break;
                        }
                    }
                }
                _ => {}
            },
            c if (c as u32) < 0x20 || c == '\u{7f}' => {}
            c => put(&mut line, &mut col, c),
        }
    }
    out.extend(line.iter().collect::<String>().trim_end().chars());
    out
}

/// Translates a key name to the bytes an xterm sends. Understands `Enter`/`Return`, `Tab`,
/// `Escape`/`Esc`, arrows, `Home`/`End`/`PageUp`/`PageDown`/`Insert`/`Delete`, `Backspace`,
/// `Space`, `F1`..`F12`, any single character, and modifiers `ctrl+` / `C-`, `alt+` / `M-`,
/// `shift+` (stacked: `ctrl+alt+Delete`). Modified special keys use xterm's parameter form
/// (ctrl+Up = ESC [ 1 ; 5 A).
pub fn key_to_bytes(name: &str) -> Result<Vec<u8>> {
    let (mut ctrl, mut alt, mut shift) = (false, false, false);
    let mut rest = name;
    loop {
        let lower = rest.to_ascii_lowercase();
        let strip = |p: &str| lower.strip_prefix(p).map(|r| r.len());
        if let Some(n) = strip("ctrl+").or_else(|| strip("c-")).or_else(|| strip("control+")) {
            ctrl = true;
            rest = &rest[rest.len() - n..];
        } else if let Some(n) = strip("alt+").or_else(|| strip("m-")).or_else(|| strip("meta+")) {
            alt = true;
            rest = &rest[rest.len() - n..];
        } else if let Some(n) = strip("shift+").or_else(|| strip("s-")) {
            shift = true;
            rest = &rest[rest.len() - n..];
        } else {
            break;
        }
    }
    if rest.is_empty() {
        return Err(err!("no key after the modifiers in \"{name}\""));
    }
    let modifier = 1 + shift as u8 + 2 * alt as u8 + 4 * ctrl as u8;

    // Keys with a CSI form: (final, number for the ~ form).
    let csi: Option<(u8, Option<u8>)> = match rest.to_ascii_lowercase().as_str() {
        "up" => Some((b'A', None)),
        "down" => Some((b'B', None)),
        "right" => Some((b'C', None)),
        "left" => Some((b'D', None)),
        "home" => Some((b'H', None)),
        "end" => Some((b'F', None)),
        "insert" | "ins" => Some((b'~', Some(2))),
        "delete" | "del" => Some((b'~', Some(3))),
        "pageup" | "pgup" => Some((b'~', Some(5))),
        "pagedown" | "pgdn" => Some((b'~', Some(6))),
        _ => None,
    };
    if let Some((fin, num)) = csi {
        let mut out = b"\x1b[".to_vec();
        match (num, modifier) {
            (Some(n), 1) => out.extend_from_slice(n.to_string().as_bytes()),
            (Some(n), m) => out.extend_from_slice(format!("{n};{m}").as_bytes()),
            (None, 1) => {}
            (None, m) => out.extend_from_slice(format!("1;{m}").as_bytes()),
        }
        out.push(fin);
        return Ok(out);
    }
    let lower = rest.to_ascii_lowercase();
    if let Some(n) = lower.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) {
        let base = function_key(n)?;
        if modifier == 1 {
            return Ok(base);
        }
        // F1-F4 are SS3 P..S: modified, CSI 1;m P..S; the rest CSI n;m ~.
        return Ok(if n <= 4 {
            format!("\x1b[1;{modifier}{}", base[2] as char).into_bytes()
        } else {
            let num = String::from_utf8_lossy(&base[2..base.len() - 1]).into_owned();
            format!("\x1b[{num};{modifier}~").into_bytes()
        });
    }
    if lower == "tab" && shift {
        return Ok(b"\x1b[Z".to_vec());
    }

    let base: Vec<u8> = match lower.as_str() {
        "enter" | "return" | "ret" | "cr" => vec![b'\r'],
        "newline" | "lf" => vec![b'\n'],
        "tab" => vec![b'\t'],
        "escape" | "esc" => vec![0x1b],
        "space" | "spc" => vec![b' '],
        "backspace" | "bs" => vec![0x7f],
        _ => {
            let chars: Vec<char> = rest.chars().collect();
            if chars.len() != 1 {
                return Err(err!("c-sh does not know the key \"{rest}\" (c-sh help: KEY names)"));
            }
            let c = if shift { chars[0].to_uppercase().next().unwrap_or(chars[0]) } else { chars[0] };
            let mut b = [0u8; 4];
            c.encode_utf8(&mut b).as_bytes().to_vec()
        }
    };
    let mut out = if ctrl { apply_ctrl(&base)? } else { base };
    if alt {
        // Alt: an ESC prefix (xterm's metaSendsEscape).
        out.insert(0, 0x1b);
    }
    Ok(out)
}

/// Control: for a letter, the control code (Ctrl-A = 1); for a few symbols their control codes.
fn apply_ctrl(bytes: &[u8]) -> Result<Vec<u8>> {
    if bytes.len() == 1 {
        let b = bytes[0];
        let code = match b {
            b'a'..=b'z' => b - b'a' + 1,
            b'A'..=b'Z' => b - b'A' + 1,
            b'@' => 0,
            b'[' => 27,
            b'\\' => 28,
            b']' => 29,
            b'^' => 30,
            b'_' => 31,
            b' ' => 0,
            b'?' => 127,
            _ => return Err(err!("ctrl+ needs a letter or @ [ \\ ] ^ _ ?")),
        };
        Ok(vec![code])
    } else {
        Err(err!("ctrl+ applies to a single key"))
    }
}

/// xterm F1..F12 sequences.
fn function_key(n: u8) -> Result<Vec<u8>> {
    Ok(match n {
        1 => b"\x1bOP".to_vec(),
        2 => b"\x1bOQ".to_vec(),
        3 => b"\x1bOR".to_vec(),
        4 => b"\x1bOS".to_vec(),
        5 => b"\x1b[15~".to_vec(),
        6 => b"\x1b[17~".to_vec(),
        7 => b"\x1b[18~".to_vec(),
        8 => b"\x1b[19~".to_vec(),
        9 => b"\x1b[20~".to_vec(),
        10 => b"\x1b[21~".to_vec(),
        11 => b"\x1b[23~".to_vec(),
        12 => b"\x1b[24~".to_vec(),
        _ => return Err(err!("there is no F{n}")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys() {
        assert_eq!(key_to_bytes("Enter").unwrap(), b"\r");
        assert_eq!(key_to_bytes("ctrl+c").unwrap(), vec![3]);
        assert_eq!(key_to_bytes("C-c").unwrap(), vec![3]);
        assert_eq!(key_to_bytes("Up").unwrap(), b"\x1b[A");
        assert_eq!(key_to_bytes("alt+x").unwrap(), b"\x1bx");
        assert_eq!(key_to_bytes("ctrl+alt+Delete").unwrap(), b"\x1b[3;7~");
        assert_eq!(key_to_bytes("ctrl+alt+x").unwrap(), b"\x1b\x18");
        assert_eq!(key_to_bytes("ctrl+Up").unwrap(), b"\x1b[1;5A");
        assert_eq!(key_to_bytes("shift+Tab").unwrap(), b"\x1b[Z");
        assert_eq!(key_to_bytes("shift+F5").unwrap(), b"\x1b[15;2~");
        assert_eq!(key_to_bytes("ctrl+F1").unwrap(), b"\x1b[1;5P");
        assert_eq!(key_to_bytes("ctrl+Space").unwrap(), vec![0]);
        assert_eq!(key_to_bytes("F5").unwrap(), b"\x1b[15~");
        assert_eq!(key_to_bytes("a").unwrap(), b"a");
        assert_eq!(key_to_bytes("한").unwrap(), "한".as_bytes());
        assert!(key_to_bytes("nope").is_err());
    }

    #[test]
    fn plain_text() {
        assert_eq!(plain(b"a\x1b[1;31mred\x1b[0m b\r\n"), "ared b\n");
        assert_eq!(plain(b"10%\r50%\r100%\n"), "100%\n");
        assert_eq!(plain(b"\x1b]0;title\x07x\x1b]3008;start=1\x1b\\y\n"), "xy\n");
        assert_eq!(plain(b"abc\x08\x08X\n"), "aXc\n");
        assert_eq!(plain(b"marker\r\x1b[2Kafter"), "after");
        assert_eq!(plain(b"\x1b[?2004lok\x1b(Bx"), "okx");
        assert_eq!(plain("한글\n".as_bytes()), "한글\n");
    }

    #[test]
    fn transcript_and_screen() {
        let mut t = Terminal::new(20, 3);
        t.feed(b"hello\r\nworld");
        assert!(t.screen().contains("hello"));
        assert!(t.screen().contains("world"));
        let (data, end, lost) = t.since(0);
        assert_eq!(data, b"hello\r\nworld");
        assert_eq!(end, 12);
        assert!(!lost);
        let (data2, _, _) = t.since(end);
        assert!(data2.is_empty());
    }
}

//! The Telnet protocol (RFC 854) without the socket: what goes out for what
//! comes in, byte by byte, so every case is tested here. The transport only
//! moves bytes.
//!
//! Options are agreed as in RFC 1143, simplified because this end never
//! turns an option off: an option is off, asked for, or on, and a request
//! is answered only when it changes something, so two ends cannot loop.
//!
//! This end asks for its options only once the server has negotiated
//! (PuTTY's passive mode): some servers echo, or choke on, option bytes
//! that come before they have set up the line (JPL Horizons does).

const IAC: u8 = 255;
const DONT: u8 = 254;
const DO: u8 = 253;
const WONT: u8 = 252;
const WILL: u8 = 251;
const SB: u8 = 250;
const BRK: u8 = 243;
const SE: u8 = 240;

const BINARY: u8 = 0;
const ECHO: u8 = 1;
const SGA: u8 = 3;
const TTYPE: u8 = 24;
const NAWS: u8 = 31;

/// TTYPE's subcommands: the server asks with SEND, the answer is IS.
const TTYPE_IS: u8 = 0;
const TTYPE_SEND: u8 = 1;

/// What the terminal emulates, as `TERM` says it over SSH.
const TERMINAL_TYPE: &[u8] = b"XTERM-256COLOR";

/// A subnegotiation longer than this is not one of ours: the rest is
/// dropped rather than kept.
const SB_LIMIT: usize = 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Opt {
    #[default]
    No,
    /// Asked for, not answered yet.
    WantYes,
    Yes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Parse {
    Data,
    Iac,
    /// IAC and a DO, DONT, WILL or WONT: the option comes next.
    Verb(u8),
    Sb,
    SbIac,
}

/// Options this end performs: the window size and terminal type it
/// reports, no go-aheads, and 8-bit data.
fn ours(option: u8) -> bool {
    matches!(option, NAWS | TTYPE | SGA | BINARY)
}

/// Options the server may perform: no go-aheads, its echo of what is typed
/// (so this end never echoes), and 8-bit data.
fn theirs(option: u8) -> bool {
    matches!(option, SGA | ECHO | BINARY)
}

/// One Telnet connection's state, from the first byte to the last.
pub(crate) struct TelnetSession {
    /// Options this end performs.
    us: [Opt; 256],
    /// Options the server performs.
    him: [Opt; 256],
    parse: Parse,
    sb: Vec<u8>,
    /// The last data byte in was CR: a NUL after it is the NVT's way of
    /// sending a bare CR, not data.
    cr_in: bool,
    /// The server has sent a CR followed by something else than NUL or
    /// LF: it does not add the NVT's NUL (busybox's telnetd), and a NUL
    /// after a CR is data. Without this, a ZMODEM download loses them.
    bare_cr: bool,
    /// The last byte out was a CR sent as CR LF: an LF or NUL right after
    /// it (a pasted CR LF) is not a second line.
    cr_out: bool,
    columns: u16,
    rows: u16,
    /// This end's own requests went out, after the server's first.
    asked: bool,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Received {
    /// What the terminal shows.
    pub data: Vec<u8>,
    /// What goes back to the server: answers to its requests.
    pub reply: Vec<u8>,
}

impl TelnetSession {
    pub fn new(columns: u16, rows: u16) -> Self {
        Self {
            us: [Opt::No; 256],
            him: [Opt::No; 256],
            parse: Parse::Data,
            sb: Vec::new(),
            cr_in: false,
            bare_cr: false,
            cr_out: false,
            columns,
            rows,
            asked: false,
        }
    }

    /// What arrived: the data, and the answers to send back. The first
    /// read the server negotiates in also carries this end's own requests.
    pub fn receive(&mut self, bytes: &[u8]) -> Received {
        let mut received = Received::default();
        let mut negotiated = false;
        for &byte in bytes {
            match self.parse {
                Parse::Data if byte == IAC => self.parse = Parse::Iac,
                Parse::Data => self.data(byte, &mut received.data),
                Parse::Iac => {
                    self.parse = Parse::Data;
                    match byte {
                        IAC => self.data(IAC, &mut received.data),
                        DO | DONT | WILL | WONT => self.parse = Parse::Verb(byte),
                        SB => {
                            self.sb.clear();
                            self.parse = Parse::Sb;
                        }
                        // NOP, GA, DM, a stray SE: nothing for the screen.
                        _ => {}
                    }
                }
                Parse::Verb(verb) => {
                    self.parse = Parse::Data;
                    self.negotiate(verb, byte, &mut received.reply);
                    negotiated = true;
                }
                Parse::Sb if byte == IAC => self.parse = Parse::SbIac,
                Parse::Sb => self.keep_sb(byte),
                Parse::SbIac => match byte {
                    SE => {
                        self.parse = Parse::Data;
                        self.subnegotiation(&mut received.reply);
                    }
                    IAC => {
                        self.parse = Parse::Sb;
                        self.keep_sb(IAC);
                    }
                    // Not a subnegotiation after all: drop it.
                    _ => self.parse = Parse::Data,
                },
            }
        }
        if negotiated && !std::mem::replace(&mut self.asked, true) {
            received.reply.extend(self.requests());
        }
        received
    }

    /// What is typed, as it goes on the wire: IAC doubled, and Enter's CR as
    /// the NVT's newline, CR LF (PuTTY's default), unless the data is binary.
    pub fn send(&mut self, bytes: &[u8]) -> Vec<u8> {
        let binary = self.us[BINARY as usize] == Opt::Yes;
        let mut out = Vec::with_capacity(bytes.len() + 2);
        for &byte in bytes {
            if std::mem::take(&mut self.cr_out) && matches!(byte, b'\n' | 0) {
                continue;
            }
            match byte {
                IAC => out.extend([IAC, IAC]),
                b'\r' if !binary => {
                    out.extend(*b"\r\n");
                    self.cr_out = true;
                }
                _ => out.push(byte),
            }
        }
        out
    }

    /// The window's new size, sent once the server takes window sizes.
    pub fn resize(&mut self, columns: u16, rows: u16) -> Vec<u8> {
        self.columns = columns;
        self.rows = rows;
        if self.us[NAWS as usize] == Opt::Yes {
            self.window_size()
        } else {
            Vec::new()
        }
    }

    /// Whether the server echoes what is typed. Until it agrees to, this
    /// end shows the typing itself (`LocalLine`).
    pub fn server_echoes(&self) -> bool {
        self.him[ECHO as usize] == Opt::Yes
    }

    /// 发送 Break: Telnet's own break, which console servers pass on to
    /// the serial line.
    pub fn brk(&self) -> Vec<u8> {
        vec![IAC, BRK]
    }

    /// This end's requests, as PuTTY's: it reports the window size and
    /// terminal type, wants no go-aheads either way, and wants the server
    /// to echo. What the server already asked about is not asked again.
    fn requests(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        for (verb, option) in [
            (WILL, NAWS),
            (WILL, TTYPE),
            (DO, SGA),
            (WILL, SGA),
            (DO, ECHO),
        ] {
            let state = match verb {
                WILL => &mut self.us[option as usize],
                _ => &mut self.him[option as usize],
            };
            if *state == Opt::No {
                *state = Opt::WantYes;
                out.extend([IAC, verb, option]);
            }
        }
        out
    }

    fn data(&mut self, byte: u8, data: &mut Vec<u8>) {
        if std::mem::take(&mut self.cr_in) {
            if byte == 0 && !self.bare_cr && self.him[BINARY as usize] != Opt::Yes {
                return;
            }
            self.bare_cr |= byte != 0 && byte != b'\n';
        }
        self.cr_in = byte == b'\r';
        data.push(byte);
    }

    fn keep_sb(&mut self, byte: u8) {
        if self.sb.len() < SB_LIMIT {
            self.sb.push(byte);
        }
    }

    fn negotiate(&mut self, verb: u8, option: u8, reply: &mut Vec<u8>) {
        let index = option as usize;
        match verb {
            DO => match self.us[index] {
                Opt::Yes => {}
                Opt::WantYes => {
                    self.us[index] = Opt::Yes;
                    self.enabled(option, reply);
                }
                Opt::No if ours(option) => {
                    self.us[index] = Opt::Yes;
                    reply.extend([IAC, WILL, option]);
                    self.enabled(option, reply);
                }
                Opt::No => reply.extend([IAC, WONT, option]),
            },
            DONT => match self.us[index] {
                Opt::Yes => {
                    self.us[index] = Opt::No;
                    reply.extend([IAC, WONT, option]);
                }
                Opt::WantYes => self.us[index] = Opt::No,
                Opt::No => {}
            },
            WILL => match self.him[index] {
                Opt::Yes => {}
                Opt::WantYes => self.him[index] = Opt::Yes,
                Opt::No if theirs(option) => {
                    self.him[index] = Opt::Yes;
                    reply.extend([IAC, DO, option]);
                }
                Opt::No => reply.extend([IAC, DONT, option]),
            },
            // WONT
            _ => match self.him[index] {
                Opt::Yes => {
                    self.him[index] = Opt::No;
                    reply.extend([IAC, DONT, option]);
                }
                Opt::WantYes => self.him[index] = Opt::No,
                Opt::No => {}
            },
        }
    }

    /// An option of ours just turned on: the window size goes at once.
    fn enabled(&self, option: u8, reply: &mut Vec<u8>) {
        if option == NAWS {
            reply.extend(self.window_size());
        }
    }

    fn subnegotiation(&mut self, reply: &mut Vec<u8>) {
        if self.sb.as_slice() == [TTYPE, TTYPE_SEND] && self.us[TTYPE as usize] == Opt::Yes {
            reply.extend([IAC, SB, TTYPE, TTYPE_IS]);
            reply.extend(TERMINAL_TYPE);
            reply.extend([IAC, SE]);
        }
    }

    fn window_size(&self) -> Vec<u8> {
        let mut out = vec![IAC, SB, NAWS];
        for value in [self.columns, self.rows] {
            for byte in value.to_be_bytes() {
                out.push(byte);
                if byte == IAC {
                    out.push(IAC);
                }
            }
        }
        out.extend([IAC, SE]);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A session whose own requests went out.
    fn started() -> TelnetSession {
        let mut session = TelnetSession::new(80, 24);
        session.requests();
        session.asked = true;
        session
    }

    fn reply(session: &mut TelnetSession, bytes: &[u8]) -> Vec<u8> {
        let received = session.receive(bytes);
        assert!(received.data.is_empty(), "{:?}", received.data);
        received.reply
    }

    #[test]
    fn it_asks_like_putty_once_the_server_negotiates() {
        let mut session = TelnetSession::new(80, 24);
        let received = session.receive(b"login: ");
        assert!(received.reply.is_empty(), "nothing before the server asks");
        assert_eq!(
            reply(&mut session, &[IAC, WONT, 200]),
            [
                IAC, WILL, NAWS, IAC, WILL, TTYPE, IAC, DO, SGA, IAC, WILL, SGA, IAC, DO, ECHO
            ]
        );
        assert!(reply(&mut session, &[IAC, WONT, 200]).is_empty(), "once");
    }

    #[test]
    fn what_the_server_asked_about_first_is_not_asked_again() {
        let mut session = TelnetSession::new(80, 24);
        assert_eq!(
            reply(
                &mut session,
                &[IAC, WILL, ECHO, IAC, WILL, SGA, IAC, DO, NAWS]
            ),
            [
                IAC, DO, ECHO, IAC, DO, SGA, IAC, WILL, NAWS, IAC, SB, NAWS, 0, 80, 0, 24, IAC,
                SE, // then its own, less what is settled
                IAC, WILL, TTYPE, IAC, WILL, SGA
            ]
        );
        assert!(reply(&mut session, &[IAC, DO, TTYPE, IAC, DO, SGA]).is_empty());
    }

    #[test]
    fn answers_to_its_own_requests_are_not_answered_again() {
        let mut session = started();
        assert!(reply(&mut session, &[IAC, WILL, ECHO, IAC, WILL, SGA]).is_empty());
        assert!(reply(&mut session, &[IAC, DO, SGA, IAC, DO, TTYPE]).is_empty());
        // Said again: already on, still nothing.
        assert!(reply(&mut session, &[IAC, WILL, ECHO, IAC, DO, SGA]).is_empty());
    }

    #[test]
    fn the_window_size_goes_once_agreed_and_on_every_resize() {
        let mut session = started();
        assert!(session.resize(100, 30).is_empty(), "not agreed yet");
        assert_eq!(
            reply(&mut session, &[IAC, DO, NAWS]),
            [IAC, SB, NAWS, 0, 100, 0, 30, IAC, SE]
        );
        assert_eq!(
            session.resize(255, 300),
            [IAC, SB, NAWS, 0, IAC, IAC, 1, 44, IAC, SE],
            "a 255 in the size is doubled"
        );
    }

    #[test]
    fn a_server_asking_first_gets_an_answer() {
        let mut session = TelnetSession::new(80, 24);
        assert!(
            reply(&mut session, &[IAC, DO, NAWS])
                .starts_with(&[IAC, WILL, NAWS, IAC, SB, NAWS, 0, 80, 0, 24, IAC, SE])
        );
        assert_eq!(reply(&mut session, &[IAC, DO, BINARY]), [IAC, WILL, BINARY]);
        assert_eq!(reply(&mut session, &[IAC, WILL, BINARY]), [IAC, DO, BINARY]);
    }

    #[test]
    fn options_it_does_not_know_are_refused() {
        let mut session = started();
        assert_eq!(reply(&mut session, &[IAC, DO, 39]), [IAC, WONT, 39]);
        assert_eq!(reply(&mut session, &[IAC, WILL, 34]), [IAC, DONT, 34]);
        assert!(reply(&mut session, &[IAC, DONT, 39, IAC, WONT, 34]).is_empty());
    }

    #[test]
    fn a_refusal_is_taken_without_a_loop() {
        let mut session = started();
        assert!(reply(&mut session, &[IAC, WONT, ECHO, IAC, DONT, NAWS]).is_empty());
        assert!(
            session.resize(100, 30).is_empty(),
            "no window sizes after DONT"
        );
        // Turned off after being on: acknowledged once.
        assert!(reply(&mut session, &[IAC, DO, SGA]).is_empty());
        assert_eq!(reply(&mut session, &[IAC, DONT, SGA]), [IAC, WONT, SGA]);
        assert!(reply(&mut session, &[IAC, DONT, SGA]).is_empty());
    }

    #[test]
    fn the_terminal_type_is_xterm() {
        let mut session = started();
        assert!(reply(&mut session, &[IAC, DO, TTYPE]).is_empty());
        let mut expected = vec![IAC, SB, TTYPE, TTYPE_IS];
        expected.extend(b"XTERM-256COLOR");
        expected.extend([IAC, SE]);
        assert_eq!(
            reply(&mut session, &[IAC, SB, TTYPE, TTYPE_SEND, IAC, SE]),
            expected
        );
    }

    #[test]
    fn commands_split_across_reads_are_still_commands() {
        let mut session = started();
        session.receive(&[IAC, DO, TTYPE]);
        let mut data = Vec::new();
        let mut replies = Vec::new();
        for byte in [
            b'a', IAC, SB, TTYPE, TTYPE_SEND, IAC, SE, IAC, IAC, b'b', IAC, 241,
        ] {
            let received = session.receive(&[byte]);
            data.extend(received.data);
            replies.extend(received.reply);
        }
        assert_eq!(
            data,
            [b'a', IAC, b'b'],
            "IAC IAC is one 255; NOP is nothing"
        );
        assert!(replies.starts_with(&[IAC, SB, TTYPE, TTYPE_IS]));
    }

    #[test]
    fn a_runaway_subnegotiation_is_capped_and_ends() {
        let mut session = started();
        let mut bytes = vec![IAC, SB, 39];
        bytes.extend(vec![b'x'; 5000]);
        bytes.extend([IAC, SE, b'o', b'k']);
        assert_eq!(session.receive(&bytes).data, b"ok");
    }

    #[test]
    fn a_bare_cr_arrives_without_its_nul() {
        let mut session = started();
        assert_eq!(session.receive(b"a\r\0b\r\nc").data, b"a\rb\r\nc");
        // Split between reads.
        assert_eq!(session.receive(b"x\r").data, b"x\r");
        assert_eq!(session.receive(b"\0y").data, b"y");
        // In binary, a NUL is data.
        session.receive(&[IAC, WILL, BINARY]);
        assert_eq!(session.receive(b"\r\0").data, b"\r\0");
    }

    #[test]
    fn a_server_that_adds_no_nul_keeps_its_nuls() {
        let mut session = started();
        assert_eq!(session.receive(b"a\r\0b").data, b"a\rb");
        // busybox's telnetd sends a CR as it is: a progress line, say.
        assert_eq!(session.receive(b"\r50%").data, b"\r50%");
        assert_eq!(session.receive(b"\r\0").data, b"\r\0");
    }

    #[test]
    fn enter_goes_as_cr_lf_once() {
        let mut session = started();
        assert_eq!(session.send(b"ls\r"), b"ls\r\n");
        assert_eq!(
            session.send(b"a\r\nb"),
            b"a\r\nb",
            "a pasted CR LF stays one"
        );
        assert_eq!(session.send(b"\r"), b"\r\n");
        assert_eq!(
            session.send(b"\n"),
            b"",
            "the LF after it in the next write too"
        );
        assert_eq!(session.send(b"\n"), b"\n", "a lone LF is just an LF");
        assert_eq!(session.send(&[IAC]), [IAC, IAC]);
    }

    #[test]
    fn binary_data_goes_as_is() {
        let mut session = started();
        session.receive(&[IAC, DO, BINARY]);
        assert_eq!(session.send(b"ls\r"), b"ls\r");
        assert_eq!(session.send(&[IAC]), [IAC, IAC], "IAC is still doubled");
    }

    #[test]
    fn the_server_echoes_only_once_it_agrees() {
        let mut session = TelnetSession::new(80, 24);
        assert!(!session.server_echoes());
        session.receive(&[IAC, DO, NAWS]);
        assert!(!session.server_echoes(), "asked, not answered");
        session.receive(&[IAC, WILL, ECHO]);
        assert!(session.server_echoes());
        session.receive(&[IAC, WONT, ECHO]);
        assert!(!session.server_echoes(), "as for a password, then back");
    }

    #[test]
    fn a_break_is_iac_brk() {
        assert_eq!(started().brk(), [IAC, BRK]);
    }
}

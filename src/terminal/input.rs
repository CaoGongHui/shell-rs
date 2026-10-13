use gpui_kit::Keystroke;

/// An interactive input, before encoding for each recipient's terminal mode.
/// Protocol replies, mouse reports, IME preedit and automated commands never
/// enter this path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalInput {
    Key(Keystroke),
    Bytes(Vec<u8>),
    Paste(String),
}

/// Input accepted by a terminal view. The connection generation prevents an
/// input queued before a reconnect from reaching the new connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalInputEvent {
    input: TerminalInput,
    generation: u64,
}

impl TerminalInputEvent {
    pub(super) fn new(input: TerminalInput, generation: u64) -> Self {
        Self { input, generation }
    }

    pub fn input(&self) -> &TerminalInput {
        &self.input
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }
}

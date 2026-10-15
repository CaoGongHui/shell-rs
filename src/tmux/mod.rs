//! tmux: the right sidebar's tool for the tmux sessions of the host of the
//! SSH terminal in front: lists them with the one the terminal is in, starts
//! one, attaches the terminal to one or detaches it, renames and ends them.
//!
//! The list comes over the terminal's own connection, beside the shell. A
//! terminal not in tmux is attached by typing `tmux attach` into it, as
//! 历史命令 types a command: only the terminal itself can hold the session.
//! A terminal already in tmux has its keys go to the program in the pane,
//! so its tmux client is told what to do beside the shell instead.

mod linux;
mod model;
mod name_dialog;
mod tmux_panel;

pub use linux::{
    attach_typed, detach_command, done, kill_command, new_command, new_typed, rename_command,
    switch_command,
};
pub use model::{TmuxSession, TmuxTable};
pub use name_dialog::{NameHandler, SessionName, open_name_dialog};
pub use tmux_panel::TmuxPanel;

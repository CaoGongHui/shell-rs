//! 项目: the right sidebar's tool beside local terminals, which keeps local
//! terminals, in groups or not, like hosts. The terminals and groups themselves
//! are in the host store (`host::SavedTerminal`); this is their tree and
//! their dialogs.

mod name_dialog;
mod outline;
mod project_panel;

pub use name_dialog::{NameDialog, confirm_delete_terminal_group, open_name_dialog};
pub use outline::{ProjectRow, project_rows};
pub use project_panel::ProjectPanel;

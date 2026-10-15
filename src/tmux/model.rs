//! What tmux said about its sessions, and what may go into a command.

/// A tmux session of the host.
#[derive(Clone, Debug, PartialEq)]
pub struct TmuxSession {
    /// tmux's own id for it, `$` and a number. Commands name the session by
    /// it, so a name with spaces or quotes in it never goes into one.
    pub id: String,
    pub name: String,
    /// How many terminals are attached to it.
    pub attached: u32,
    pub windows: u32,
    /// The folder it started in; empty from a tmux too old to say.
    pub path: String,
}

/// A reading of the host's tmux.
#[derive(Clone, Debug, PartialEq)]
pub struct TmuxTable {
    /// What `tmux -V` says: 「tmux 3.2a」.
    pub version: String,
    /// In tmux's order, by name.
    pub sessions: Vec<TmuxSession>,
    /// The SSH terminal's own tty, as tmux names its clients
    /// (`/dev/pts/3`), when it was found.
    pub tty: Option<String>,
    /// The session the SSH terminal is attached to, by id.
    pub current: Option<String>,
}

impl TmuxTable {
    pub fn session(&self, id: &str) -> Option<&TmuxSession> {
        self.sessions.iter().find(|session| session.id == id)
    }

    /// The terminal's tty while it is attached to a session. tmux then
    /// takes the keys typed into the terminal for the program in its pane,
    /// so the terminal's client is told what to do with tmux's commands
    /// instead.
    pub fn client(&self) -> Option<&str> {
        self.current.as_ref().and(self.tty.as_deref())
    }

    pub fn names(&self) -> Vec<String> {
        self.sessions
            .iter()
            .map(|session| session.name.clone())
            .collect()
    }
}

/// Whether `name` can name a new session: letters, digits, `-` and `_`,
/// not first a `-`. tmux takes nearly anything (not `:` or `.`), but a name
/// with a space or a quote would need quoting, and each login shell quotes
/// its own way; this one goes into commands and is typed as it is.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= 64
        && !name.starts_with('-')
        && name
            .chars()
            .all(|character| character.is_alphanumeric() || character == '-' || character == '_')
}

/// Whether `id` is a session's id: `$` and a number.
pub fn valid_id(id: &str) -> bool {
    id.strip_prefix('$').is_some_and(|number| {
        !number.is_empty() && number.chars().all(|digit| digit.is_ascii_digit())
    })
}

/// Whether `tty` is a client's tty: `/dev/` and letters, digits and `/`.
pub fn valid_tty(tty: &str) -> bool {
    tty.strip_prefix("/dev/").is_some_and(|name| {
        !name.is_empty()
            && name
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '/')
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_name_is_letters_digits_dashes_and_underscores() {
        for name in [
            "work",
            "train-7b",
            "my_job",
            "0",
            "训练",
            "a".repeat(64).as_str(),
        ] {
            assert!(valid_name(name), "{name}");
        }
        for name in [
            "",
            "-work",
            "my job",
            "a'b",
            "a\"b",
            "$HOME",
            "a;b",
            "a.b",
            "a:b",
            "a!b",
            "a".repeat(65).as_str(),
        ] {
            assert!(!valid_name(name), "{name}");
        }
    }

    #[test]
    fn ids_and_ttys_are_only_what_tmux_gives() {
        assert!(valid_id("$0"));
        assert!(valid_id("$12"));
        for id in ["", "$", "12", "$1a", "$1;", "@1"] {
            assert!(!valid_id(id), "{id}");
        }
        assert!(valid_tty("/dev/pts/3"));
        assert!(valid_tty("/dev/ttys003"));
        for tty in ["", "/dev/", "pts/3", "/dev/pts/3 x", "/dev/pts/$1"] {
            assert!(!valid_tty(tty), "{tty}");
        }
    }

    #[test]
    fn the_client_is_the_tty_once_attached() {
        let mut table = TmuxTable {
            version: "tmux 3.2a".into(),
            sessions: Vec::new(),
            tty: Some("/dev/pts/3".into()),
            current: None,
        };
        assert_eq!(table.client(), None);
        table.current = Some("$1".into());
        assert_eq!(table.client(), Some("/dev/pts/3"));
        table.tty = None;
        assert_eq!(table.client(), None);
    }
}

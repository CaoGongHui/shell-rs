//! Setting up a host's shell to report its folder (OSC 7) for the right
//! sidebar's 文件 to follow: what is run, on the terminal's own connection,
//! to see whether the login shell does already, and to add the few lines
//! that make it. Only bash and zsh are set up; fish reports by itself.
//!
//! Like the tools' commands, each is one line in single quotes for `sh -c`,
//! so whatever the login shell is it hands the script to `sh` untouched:
//! no single quote inside, and no `!` for csh to expand. What is written to
//! the file goes as an argument of its own, encoded for `printf %b`.

use super::run::encode_command;
use crate::i18n::t;

/// The line the setup is marked with, at its start and end, in the file it
/// goes into: how a check knows it is there, and a setup does not add it
/// twice. Never changed once released.
const MARKER: &str = "ShellRS directory follow";

/// What the check runs: the login shell, the host's name as its shell
/// reports it (`uname -n`, which bash's `$HOSTNAME` and zsh's `$HOST` are
/// too), the startup file the setup would go into, and whether that file
/// reports the folder already, each on an `@@` line.
///
/// bash's file is `~/.bashrc` when the login file it reads (the first of
/// `.bash_profile`, `.bash_login` and `.profile`) reads that too, as most
/// systems' do; else the login file itself, a new `.bash_profile` when
/// there is none.
pub fn check_command() -> String {
    format!(
        "sh -c 's=${{SHELL##*/}}; l=; f=; case $s in bash) l=.bash_profile; \
         for p in .bash_profile .bash_login .profile; do \
         if [ -f \"$HOME/$p\" ]; then l=$p; break; fi; done; f=$l; \
         if [ -f \"$HOME/$l\" ] && grep -q bashrc \"$HOME/$l\"; then f=.bashrc; fi;; \
         zsh) f=.zshrc;; esac; c=no; \
         if [ -n \"$f\" ] && [ -f \"$HOME/$f\" ] && grep -qF -e \"{MARKER}\" \
         -e \"]7;\" -e \"1337;CurrentDir\" \"$HOME/$f\"; then c=yes; fi; \
         echo \"@@shell $s\"; echo \"@@host $(uname -n)\"; echo \"@@file $f\"; \
         echo \"@@configured $c\"'"
    )
}

/// What the check found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FollowCheck {
    /// The host's name, which the folders it reports carry.
    pub hostname: String,
    /// The setup to offer; none when the shell reports already, or is not
    /// one ShellRS sets up.
    pub setup: Option<FollowSetup>,
}

/// A setup ShellRS can make: which shell, into which file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FollowSetup {
    pub shell: SetupShell,
    pub file: StartupFile,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupShell {
    Bash,
    Zsh,
}

/// The file in the home folder that a setup goes into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartupFile {
    Bashrc,
    BashProfile,
    BashLogin,
    Zshrc,
}

impl StartupFile {
    fn name(self) -> &'static str {
        match self {
            Self::Bashrc => ".bashrc",
            Self::BashProfile => ".bash_profile",
            Self::BashLogin => ".bash_login",
            Self::Zshrc => ".zshrc",
        }
    }
}

impl FollowSetup {
    /// The shell by its name, as the question names it.
    pub fn shell_name(self) -> &'static str {
        match self.shell {
            SetupShell::Bash => "bash",
            SetupShell::Zsh => "zsh",
        }
    }

    /// The file as the question names it: `~/.bashrc`.
    pub fn file_label(self) -> String {
        format!("~/{}", self.file.name())
    }

    /// What is typed into the terminal after the setup, so that the shell
    /// running there reports at once. The space keeps it out of a history
    /// that ignores such lines.
    pub fn source_line(self) -> String {
        format!(" source {}", self.file_label())
    }
}

/// What the check's output says; `None` when it is not what the check
/// prints, as where there is no `sh` to run it. The last `@@` lines count:
/// anything a startup file prints comes before them.
pub fn parse_check(output: &str) -> Option<FollowCheck> {
    let value = |key: &str| {
        let prefix = format!("@@{key} ");
        output
            .lines()
            .rev()
            .find_map(|line| line.trim_end().strip_prefix(&prefix).map(str::trim))
    };
    let hostname = value("host").filter(|host| !host.is_empty())?.to_string();
    let shell = match value("shell")? {
        "bash" => Some(SetupShell::Bash),
        "zsh" => Some(SetupShell::Zsh),
        _ => None,
    };
    let file = match value("file").unwrap_or_default() {
        ".bashrc" => Some(StartupFile::Bashrc),
        ".bash_profile" => Some(StartupFile::BashProfile),
        ".bash_login" => Some(StartupFile::BashLogin),
        ".zshrc" => Some(StartupFile::Zshrc),
        // `.profile`, which `sh` reads too: bash's lines would break it.
        _ => None,
    };
    let configured = value("configured")? == "yes";
    let setup = match (shell, file) {
        (Some(shell), Some(file)) if !configured => Some(FollowSetup { shell, file }),
        _ => None,
    };
    Some(FollowCheck { hostname, setup })
}

/// The lines a setup adds: a function that reports the folder, URL-encoded
/// as OSC 7 has it, run before each prompt. It keeps `$?` and whatever ran
/// before each prompt already, and adds itself once however often the file
/// is read.
fn snippet(shell: SetupShell) -> String {
    let (host, hook) = match shell {
        SetupShell::Bash => (
            "HOSTNAME",
            "case \";${PROMPT_COMMAND-};\" in\n  \
             *\";__shellrs_osc7;\"*) ;;\n  \
             *) PROMPT_COMMAND=\"__shellrs_osc7${PROMPT_COMMAND:+;$PROMPT_COMMAND}\" ;;\n\
             esac\n",
        ),
        SetupShell::Zsh => (
            "HOST",
            "autoload -Uz add-zsh-hook && add-zsh-hook precmd __shellrs_osc7\n",
        ),
    };
    format!(
        "\n# >>> {MARKER} >>>\n\
         # Reports the shell's folder to the terminal (OSC 7) before each prompt.\n\
         __shellrs_osc7() {{\n  \
         local rc=$? url= i ch hex LC_CTYPE=C LC_COLLATE=C LC_ALL= LANG=\n  \
         for ((i = 0; i < ${{#PWD}}; i++)); do\n    \
         ch=${{PWD:$i:1}}\n    \
         case $ch in\n      \
         [/._~A-Za-z0-9-]) url+=$ch ;;\n      \
         *) printf -v hex '%02X' \"'$ch\"; url+=\"%${{hex: -2:2}}\" ;;\n    \
         esac\n  \
         done\n  \
         printf '\\033]7;file://%s%s\\a' \"${{{host}-}}\" \"$url\"\n  \
         return $rc\n\
         }}\n\
         {hook}\
         # <<< {MARKER} <<<\n"
    )
}

/// What the setup runs: append the lines to the file, unless they are
/// there already, and say `@@installed`. When the file cannot be written
/// the shell says why, which comes back instead.
pub fn install_command(setup: FollowSetup) -> String {
    format!(
        "sh -c 'exec 2>&1; f=\"$HOME/$1\"; \
         if [ -f \"$f\" ] && grep -qF \"{MARKER}\" \"$f\"; then echo @@installed; exit 0; fi; \
         printf %b \"$2\" >> \"$f\" && echo @@installed' sh {} '{}'",
        setup.file.name(),
        encode_command(snippet(setup.shell).as_bytes())
    )
}

/// Whether the setup's output says it is done, or else why not.
pub fn installed(output: &str) -> Result<(), String> {
    if output.lines().any(|line| line.trim() == "@@installed") {
        return Ok(());
    }
    Err(output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| t!("tools.no_answer").to_string()))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::path::Path;
    use std::process::{Command, Output};

    /// Run `command` as a login shell would hand it on: through `sh -c`,
    /// with this home and this login shell.
    fn run(command: &str, home: &Path, shell: &str) -> Output {
        let mut process = Command::new("sh");
        process
            .args(["-c", command])
            .env("HOME", home)
            .env("SHELL", shell);
        let _forks = crate::testing::no_forks();
        process.output().expect("sh runs")
    }

    fn check(home: &Path, shell: &str) -> FollowCheck {
        let output = run(&check_command(), home, shell);
        parse_check(&String::from_utf8_lossy(&output.stdout)).expect("the check answers")
    }

    fn setup(shell: SetupShell, file: StartupFile) -> Option<FollowSetup> {
        Some(FollowSetup { shell, file })
    }

    /// The program, if this machine has it.
    fn program(name: &str, args: &[&str], dir: &Path) -> Option<Output> {
        let mut process = Command::new(name);
        process.args(args).current_dir(dir);
        let _forks = crate::testing::no_forks();
        process.output().ok()
    }

    #[test]
    fn the_commands_are_one_line_for_any_login_shell() {
        let install = install_command(FollowSetup {
            shell: SetupShell::Bash,
            file: StartupFile::Bashrc,
        });
        for (command, quotes) in [(check_command(), 2), (install, 4)] {
            assert!(!command.contains('\n'), "{command}");
            assert!(!command.contains('!'), "{command}");
            assert_eq!(command.matches('\'').count(), quotes, "{command}");
        }
    }

    #[test]
    fn the_check_finds_the_file_bash_or_zsh_reads_and_the_host_name() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        // No login file: a new .bash_profile.
        let found = check(home, "/bin/bash");
        assert_eq!(
            found.setup,
            setup(SetupShell::Bash, StartupFile::BashProfile)
        );
        let uname = Command::new("uname").arg("-n").output().unwrap();
        assert_eq!(
            found.hostname,
            String::from_utf8_lossy(&uname.stdout).trim()
        );
        // One that reads .bashrc.
        std::fs::write(
            home.join(".bash_profile"),
            "[ -f ~/.bashrc ] && . ~/.bashrc\n",
        )
        .unwrap();
        assert_eq!(
            check(home, "/bin/bash").setup,
            setup(SetupShell::Bash, StartupFile::Bashrc)
        );
        // One that does not: the setup goes into it.
        std::fs::write(home.join(".bash_profile"), "export EDITOR=vi\n").unwrap();
        assert_eq!(
            check(home, "/bin/bash").setup,
            setup(SetupShell::Bash, StartupFile::BashProfile)
        );
        assert_eq!(
            check(home, "/usr/local/bin/zsh").setup,
            setup(SetupShell::Zsh, StartupFile::Zshrc)
        );
        assert_eq!(check(home, "/usr/bin/fish").setup, None);
        assert_eq!(check(home, "/bin/tcsh").setup, None);
    }

    #[test]
    fn a_profile_that_does_not_read_bashrc_is_left_alone() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(home.path().join(".profile"), "PATH=$HOME/bin:$PATH\n").unwrap();
        assert_eq!(check(home.path(), "/bin/bash").setup, None);
        std::fs::write(
            home.path().join(".profile"),
            "if [ -n \"$BASH_VERSION\" ]; then . \"$HOME/.bashrc\"; fi\n",
        )
        .unwrap();
        assert_eq!(
            check(home.path(), "/bin/bash").setup,
            setup(SetupShell::Bash, StartupFile::Bashrc)
        );
    }

    #[test]
    fn a_shell_that_reports_already_is_not_set_up() {
        let home = tempfile::tempdir().unwrap();
        for existing in [
            "printf '\\e]7;file://%s%s\\a' \"$HOSTNAME\" \"$PWD\"\n",
            "printf '\\e]1337;CurrentDir=%s\\a' \"$PWD\"\n",
        ] {
            std::fs::write(home.path().join(".zshrc"), existing).unwrap();
            assert_eq!(check(home.path(), "/bin/zsh").setup, None, "{existing}");
        }
    }

    #[test]
    fn what_a_startup_file_prints_before_the_answer_does_not_matter() {
        let output = "Welcome!\n@@shell banner\n@@shell bash\n@@host web-01\n\
                      @@file .bashrc\n@@configured no\n";
        assert_eq!(
            parse_check(output),
            Some(FollowCheck {
                hostname: "web-01".into(),
                setup: setup(SetupShell::Bash, StartupFile::Bashrc),
            })
        );
        assert_eq!(
            parse_check("@@shell bash\n@@host web-01\n@@file .profile\n@@configured no\n")
                .unwrap()
                .setup,
            None
        );
        // Not the check's answer: no `sh` there.
        assert_eq!(parse_check("'sh' is not recognized"), None);
        assert_eq!(parse_check("@@shell bash\n@@file .bashrc\n"), None);
    }

    #[test]
    fn the_setup_adds_its_lines_once_and_says_why_it_cannot() {
        let home = tempfile::tempdir().unwrap();
        let bash = FollowSetup {
            shell: SetupShell::Bash,
            file: StartupFile::Bashrc,
        };
        std::fs::write(home.path().join(".bashrc"), "alias ll='ls -l'\n").unwrap();
        for _ in 0..2 {
            let output = run(&install_command(bash), home.path(), "/bin/bash");
            assert_eq!(installed(&String::from_utf8_lossy(&output.stdout)), Ok(()));
        }
        assert_eq!(
            std::fs::read_to_string(home.path().join(".bashrc")).unwrap(),
            format!("alias ll='ls -l'\n{}", snippet(SetupShell::Bash))
        );
        // Now the check finds it there, in the file the login reads.
        std::fs::write(home.path().join(".bash_profile"), ". ~/.bashrc\n").unwrap();
        assert_eq!(check(home.path(), "/bin/bash").setup, None);

        let nowhere = home.path().join("missing");
        let output = run(&install_command(bash), &nowhere, "/bin/bash");
        let why = installed(&String::from_utf8_lossy(&output.stdout)).unwrap_err();
        assert!(why.contains("missing"), "{why}");
        assert_eq!(installed(""), Err(t!("tools.no_answer").to_string()));
    }

    #[test]
    fn bash_reports_the_encoded_folder_before_each_prompt() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("a b").join("文件");
        std::fs::create_dir_all(&folder).unwrap();
        let file = root.path().join("snippet");
        std::fs::write(&file, snippet(SetupShell::Bash)).unwrap();
        let script = format!(
            "PROMPT_COMMAND='echo old'; . '{file}'; . '{file}'; \
             echo \"hooks=$PROMPT_COMMAND\"; cd '{folder}'; false; __shellrs_osc7; \
             echo \"status=$?\"",
            file = file.display(),
            folder = folder.display()
        );
        let Some(output) = program(
            "bash",
            &["--noprofile", "--norc", "-c", &script],
            root.path(),
        ) else {
            return;
        };
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(text.contains("hooks=__shellrs_osc7;echo old\n"), "{text:?}");
        assert!(text.contains("status=1"), "{text:?}");
        let encoded = "/a%20b/%E6%96%87%E4%BB%B6\x07";
        assert!(text.contains("\x1b]7;file://"), "{text:?}");
        assert!(text.contains(encoded), "{text:?}");
    }

    #[test]
    fn zsh_reports_the_encoded_folder_before_each_prompt() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("a b").join("文件");
        std::fs::create_dir_all(&folder).unwrap();
        let file = root.path().join("snippet");
        std::fs::write(&file, snippet(SetupShell::Zsh)).unwrap();
        let script = format!(
            "source '{file}'; source '{file}'; print \"hooks=$precmd_functions\"; \
             cd '{folder}'; false; __shellrs_osc7; print \"status=$?\"",
            file = file.display(),
            folder = folder.display()
        );
        let Some(output) = program("zsh", &["-f", "-c", &script], root.path()) else {
            return;
        };
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(text.contains("hooks=__shellrs_osc7\n"), "{text:?}");
        assert!(text.contains("status=1"), "{text:?}");
        assert!(text.contains("/a%20b/%E6%96%87%E4%BB%B6\x07"), "{text:?}");
    }

    #[test]
    fn the_question_names_the_shell_and_its_file() {
        let setup = FollowSetup {
            shell: SetupShell::Zsh,
            file: StartupFile::Zshrc,
        };
        assert_eq!(setup.shell_name(), "zsh");
        assert_eq!(setup.file_label(), "~/.zshrc");
        assert_eq!(setup.source_line(), " source ~/.zshrc");
    }
}

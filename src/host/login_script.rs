//! 登录脚本: an SSH host's 「等待并发送」 rules, as Xshell's 「登录脚本」 has
//! them. Once the shell is up, each rule in turn waits for what the server
//! prints (`Opt>`) and types its answer (`10.0.0.5`, Enter). A bastion host
//! that asks for the asset and then its password after the login is the
//! case it is for.
//!
//! The rules are saved whole, as JSON, in one keychain entry of the host's
//! (`SecretRef::LoginScript`): an answer is often a password, and the
//! database never holds one. This module is pure, so the rules are tested
//! here; the SSH transport runs [`LoginScript`] on what the shell prints.

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// One 「等待并发送」 rule.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoginRule {
    /// What the line the cursor is on ends with, ignoring case and the
    /// blanks after it. Empty sends at once: the first output settling.
    #[serde(default)]
    pub expect: String,
    /// What is typed, Enter after it. May be empty: Enter alone.
    #[serde(default)]
    pub send: String,
}

/// What separates a rule's 等待 from its 发送 in the form's text.
pub const RULE_SEPARATOR: &str = "=>";

/// The most rules a host keeps, and the longest text of one: the whole
/// script lives in a keychain entry, which Windows caps at 2560 bytes.
pub const MAX_RULES: usize = 16;
pub const MAX_RULE_LEN: usize = 256;

/// The rules as the form shows them: `等待 => 发送`, one per line.
pub fn format_rules(rules: &[LoginRule]) -> String {
    rules
        .iter()
        .map(|rule| {
            if rule.send.is_empty() {
                format!("{} {RULE_SEPARATOR}", rule.expect)
            } else {
                format!("{} {RULE_SEPARATOR} {}", rule.expect, rule.send)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Why the form's text is not a script.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RulesError {
    /// The line, counted from 1, has no `=>`.
    NoSeparator(usize),
    TooMany,
    /// The line, counted from 1, is longer than [`MAX_RULE_LEN`].
    TooLong(usize),
}

/// The form's text as rules: `等待 => 发送` per line, blank lines skipped.
/// The blanks around each side go; those inside stay. The first `=>` is
/// the separator, so an answer may hold one itself.
pub fn parse_rules(text: &str) -> Result<Vec<LoginRule>, RulesError> {
    let mut rules = Vec::new();
    for (ix, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        if line.len() > MAX_RULE_LEN {
            return Err(RulesError::TooLong(ix + 1));
        }
        let Some((expect, send)) = line.split_once(RULE_SEPARATOR) else {
            return Err(RulesError::NoSeparator(ix + 1));
        };
        rules.push(LoginRule {
            expect: expect.trim().to_string(),
            send: send.trim().to_string(),
        });
        if rules.len() > MAX_RULES {
            return Err(RulesError::TooMany);
        }
    }
    Ok(rules)
}

/// The rules as the keychain keeps them. `None` for none at all: the entry
/// is deleted rather than holding an empty list.
pub fn encode_rules(rules: &[LoginRule]) -> Option<String> {
    (!rules.is_empty()).then(|| serde_json::to_string(rules).expect("rules always serialize"))
}

/// The rules read back from the keychain. Anything that does not read is
/// no script: the user is asked for nothing, and types the answers.
pub fn decode_rules(text: &str) -> Vec<LoginRule> {
    serde_json::from_str(text).unwrap_or_default()
}

/// Only the end of the line matters: a longer one is cut from the front.
const LINE_LIMIT: usize = 512;

/// Where the output is in an escape sequence, which is not text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Escape {
    None,
    Esc,
    Csi,
    /// OSC, DCS and the like, up to BEL or ST.
    String,
    StringEsc,
}

/// The rules running on what a shell prints. Each answers once, in order;
/// the script is done after the last, or as soon as the user types.
pub struct LoginScript {
    /// The rules still to answer, the next one first, with their texts
    /// lowercased once for matching.
    rules: Vec<(String, Zeroizing<String>)>,
    next: usize,
    /// The line the cursor is on, without escape sequences.
    line: Vec<u8>,
    escape: Escape,
    /// Whether any output came at all: a rule waiting for nothing answers
    /// once the first output settles.
    seen_output: bool,
    done: bool,
}

impl LoginScript {
    pub fn new(rules: Vec<LoginRule>) -> Self {
        let done = rules.is_empty();
        Self {
            rules: rules
                .into_iter()
                .map(|rule| (rule.expect.trim().to_lowercase(), Zeroizing::new(rule.send)))
                .collect(),
            next: 0,
            line: Vec::new(),
            escape: Escape::None,
            seen_output: false,
            done,
        }
    }

    /// What the shell printed.
    pub fn output(&mut self, data: &[u8]) {
        if self.done {
            return;
        }
        if !data.is_empty() {
            self.seen_output = true;
        }
        for &byte in data {
            match self.escape {
                Escape::None => match byte {
                    0x1b => self.escape = Escape::Esc,
                    b'\r' | b'\n' => self.line.clear(),
                    0x08 => {
                        self.line.pop();
                    }
                    0x00..=0x1f | 0x7f => {}
                    _ => self.line.push(byte),
                },
                Escape::Esc => {
                    self.escape = match byte {
                        b'[' => Escape::Csi,
                        b']' | b'P' | b'X' | b'^' | b'_' => Escape::String,
                        // `ESC ( B` and the like: more to come.
                        0x20..=0x2f => Escape::Esc,
                        _ => Escape::None,
                    }
                }
                Escape::Csi => {
                    if (0x40..=0x7e).contains(&byte) {
                        self.escape = Escape::None;
                    }
                }
                Escape::String => match byte {
                    0x07 => self.escape = Escape::None,
                    0x1b => self.escape = Escape::StringEsc,
                    _ => {}
                },
                Escape::StringEsc => {
                    self.escape = if byte == b'\\' {
                        Escape::None
                    } else {
                        Escape::String
                    }
                }
            }
        }
        if self.line.len() > LINE_LIMIT {
            self.line.drain(..self.line.len() - LINE_LIMIT);
        }
    }

    /// Whether the line ends with what the next rule waits for.
    pub fn waiting(&self) -> bool {
        if self.done {
            return false;
        }
        let Some((expect, _)) = self.rules.get(self.next) else {
            return false;
        };
        if expect.is_empty() {
            return self.seen_output;
        }
        let line = String::from_utf8_lossy(&self.line);
        line.trim_end().to_lowercase().ends_with(expect.as_str())
    }

    /// What to type for the rule the line answers, Enter included, and
    /// move on to the next. `None` when the line answers none.
    pub fn answer(&mut self) -> Option<Zeroizing<Vec<u8>>> {
        if !self.waiting() {
            return None;
        }
        let (_, send) = &self.rules[self.next];
        let mut bytes = Zeroizing::new(Vec::with_capacity(send.len() + 1));
        bytes.extend_from_slice(send.as_bytes());
        bytes.push(b'\r');
        self.next += 1;
        self.line.clear();
        if self.next >= self.rules.len() {
            self.finish();
        }
        Some(bytes)
    }

    /// What the user typed: they take over from here. Writes that start
    /// with ESC are the terminal answering the server (cursor reports,
    /// colors), or arrow keys, not a login.
    pub fn typed(&mut self, bytes: &[u8]) {
        if bytes.first().is_some_and(|byte| *byte != 0x1b) {
            self.finish();
        }
    }

    pub fn is_done(&self) -> bool {
        self.done
    }

    fn finish(&mut self) {
        self.done = true;
        self.rules.clear();
        self.line.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(expect: &str, send: &str) -> LoginRule {
        LoginRule {
            expect: expect.into(),
            send: send.into(),
        }
    }

    fn answer(script: &mut LoginScript) -> Option<String> {
        script
            .answer()
            .map(|bytes| String::from_utf8(bytes.to_vec()).unwrap())
    }

    #[test]
    fn a_bastion_asks_for_the_asset_then_its_password() {
        let mut script = LoginScript::new(vec![
            rule("Opt>", "10.0.0.5"),
            rule("password:", "hunter2"),
        ]);
        script.output(b"\x1b[H\x1b[2J\t\tadmin,  JumpServer\r\n\r\n\t 1) Enter IP\r\n");
        assert!(!script.waiting());
        script.output(b"Opt> \x1b[K");
        assert_eq!(answer(&mut script).as_deref(), Some("10.0.0.5\r"));
        assert!(!script.waiting(), "the echoed asset is not a prompt");
        script.output(b"10.0.0.5\r\nadmin's password: ");
        assert_eq!(answer(&mut script).as_deref(), Some("hunter2\r"));
        assert!(script.is_done());
        script.output(b"\r\nOpt> ");
        assert!(!script.waiting(), "each rule answers once");
    }

    #[test]
    fn rules_answer_in_order_only() {
        let mut script = LoginScript::new(vec![rule("Opt>", "10.0.0.1"), rule("password:", "pw")]);
        script.output(b"password: ");
        assert!(!script.waiting(), "the first rule has not answered yet");
        script.output(b"\r\nOpt>");
        assert_eq!(answer(&mut script).as_deref(), Some("10.0.0.1\r"));
    }

    #[test]
    fn prompts_match_in_any_case_through_colors_and_pieces() {
        let mut script = LoginScript::new(vec![rule("Password:", "pw")]);
        script.output(b"\x1b]0;title\x07\x1b(B\x1b[1mPASS");
        assert!(!script.waiting());
        script.output(b"WORD:\x1b[0m  ");
        assert_eq!(answer(&mut script).as_deref(), Some("pw\r"));
    }

    #[test]
    fn a_rule_waiting_for_nothing_answers_the_first_output() {
        let mut script = LoginScript::new(vec![rule("", "sudo -i"), rule("$", "")]);
        assert!(!script.waiting(), "nothing printed yet");
        script.output(b"Last login: today\r\n");
        assert_eq!(answer(&mut script).as_deref(), Some("sudo -i\r"));
        script.output(b"[root@web ~]$");
        assert_eq!(answer(&mut script).as_deref(), Some("\r"), "Enter alone");
        assert!(script.is_done());
    }

    #[test]
    fn typing_takes_over_but_terminal_replies_do_not() {
        let mut script = LoginScript::new(vec![rule("Opt>", "x")]);
        script.typed(b"\x1b[?1;2c");
        script.typed(b"\x1b[A");
        assert!(!script.is_done());
        script.typed(b"q");
        assert!(script.is_done());
        script.output(b"Opt>");
        assert!(!script.waiting());
        assert_eq!(answer(&mut script), None);
    }

    #[test]
    fn no_rules_is_done_from_the_start() {
        let script = LoginScript::new(Vec::new());
        assert!(script.is_done());
    }

    #[test]
    fn the_form_text_round_trips() {
        let rules = vec![rule("Opt>", "10.0.0.5"), rule("password:", "a => b")];
        let text = format_rules(&rules);
        assert_eq!(text, "Opt> => 10.0.0.5\npassword: => a => b");
        assert_eq!(parse_rules(&text).unwrap(), rules);
        assert_eq!(
            parse_rules("  Opt>  =>  10.0.0.1  \n\n$ =>\n").unwrap(),
            vec![rule("Opt>", "10.0.0.1"), rule("$", "")]
        );
        assert_eq!(format_rules(&[rule("$", "")]), "$ =>");
    }

    #[test]
    fn the_form_text_says_what_is_wrong() {
        assert_eq!(
            parse_rules("Opt> => 1\npassword: pw"),
            Err(RulesError::NoSeparator(2))
        );
        let long = format!("a => {}", "x".repeat(MAX_RULE_LEN));
        assert_eq!(parse_rules(&long), Err(RulesError::TooLong(1)));
        let many = "a => b\n".repeat(MAX_RULES + 1);
        assert_eq!(parse_rules(&many), Err(RulesError::TooMany));
        assert_eq!(parse_rules("  \n "), Ok(Vec::new()));
    }

    #[test]
    fn the_keychain_text_round_trips_and_forgives() {
        let rules = vec![rule("Opt>", "1"), rule("password:", "pw")];
        let text = encode_rules(&rules).unwrap();
        assert_eq!(decode_rules(&text), rules);
        assert_eq!(encode_rules(&[]), None);
        assert_eq!(decode_rules("not json"), Vec::new());
        assert_eq!(
            decode_rules(r#"[{"expect":"Opt>"}]"#),
            vec![rule("Opt>", "")]
        );
    }
}

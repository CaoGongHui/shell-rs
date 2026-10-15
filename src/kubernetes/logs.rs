//! A bounded latest-window buffer. Highlight tokens are cached independently
//! of the theme; virtual rows resolve their colors only when visible.
use std::ops::Range;

pub(super) const MAX_BYTES: usize = 512 * 1024;
pub(super) const MAX_LINES: usize = 10_000;
const DISPLAY_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Token {
    Timestamp,
    Debug,
    Info,
    Warning,
    Error,
    Key,
    String,
    Number,
    Literal,
}

pub(super) struct LogLine {
    pub text: String,
    pub raw: String,
    pub spans: Vec<(Range<usize>, Token)>,
}

impl LogLine {
    pub fn new(line: &str) -> Self {
        let display = super::format::log_line(line);
        let end = boundary(&display, DISPLAY_BYTES);
        let mut text = display[..end].to_owned();
        if end < display.len() {
            text.push('…');
        }
        let spans = tokens(&text);
        Self {
            text,
            raw: line.to_owned(),
            spans,
        }
    }
}

fn boundary(text: &str, limit: usize) -> usize {
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    end
}

/// Keep complete newest lines when possible, while even a single enormous
/// line stays bounded and valid UTF-8. No history grows across refreshes.
pub(super) fn bounded(text: String) -> (String, bool) {
    let mut start = text.len().saturating_sub(MAX_BYTES);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    if start > 0
        && text.as_bytes()[start - 1] != b'\n'
        && let Some(newline) = text[start..].find('\n')
        && start + newline + 1 < text.len()
    {
        start += newline + 1;
    }
    let lines = text[start..].lines().count();
    if lines > MAX_LINES {
        let skip = lines - MAX_LINES;
        start += text[start..]
            .split_inclusive('\n')
            .take(skip)
            .map(str::len)
            .sum::<usize>();
    }
    let clipped = start > 0 || text.len() >= MAX_BYTES;
    (text[start..].to_owned(), clipped)
}

fn level(word: &str) -> Option<Token> {
    if word.eq_ignore_ascii_case("error")
        || word.eq_ignore_ascii_case("fatal")
        || word.eq_ignore_ascii_case("panic")
    {
        Some(Token::Error)
    } else if word.eq_ignore_ascii_case("warn") || word.eq_ignore_ascii_case("warning") {
        Some(Token::Warning)
    } else if word.eq_ignore_ascii_case("info") {
        Some(Token::Info)
    } else if word.eq_ignore_ascii_case("debug") || word.eq_ignore_ascii_case("trace") {
        Some(Token::Debug)
    } else {
        None
    }
}

/// Linear lexical scan: never executes a user regex or parses an unbounded
/// JSON document. Quoted/escaped strings and Unicode use byte-safe ranges.
pub(super) fn tokens(text: &str) -> Vec<(Range<usize>, Token)> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut ix = 0;
    if bytes.len() >= 20 && bytes[4] == b'-' && bytes[7] == b'-' && bytes[10] == b'T' {
        ix = text.find(char::is_whitespace).unwrap_or(text.len());
        spans.push((0..ix, Token::Timestamp));
    }
    while ix < bytes.len() {
        let start = ix;
        if bytes[ix] == b'"' {
            ix += 1;
            while ix < bytes.len() {
                if bytes[ix] == b'\\' {
                    ix = (ix + 2).min(bytes.len());
                } else if bytes[ix] == b'"' {
                    ix += 1;
                    break;
                } else {
                    ix += 1;
                }
            }
            // An escape can only split a Unicode codepoint in malformed data.
            while !text.is_char_boundary(ix) {
                ix += 1;
            }
            let quoted = &text[start..ix];
            let token = if text[ix..].trim_start().starts_with(':') {
                Token::Key
            } else {
                level(quoted.trim_matches('"')).unwrap_or(Token::String)
            };
            spans.push((start..ix, token));
        } else if bytes[ix].is_ascii_alphanumeric() || bytes[ix] == b'_' || bytes[ix] >= 128 {
            ix += 1;
            while ix < bytes.len()
                && (bytes[ix].is_ascii_alphanumeric()
                    || bytes[ix] == b'_'
                    || bytes[ix] >= 128
                    || bytes[ix] == b'.')
            {
                ix += 1;
            }
            let word = &text[start..ix];
            let token = level(word).or_else(|| {
                if matches!(word, "true" | "false" | "null") {
                    Some(Token::Literal)
                } else if word.parse::<f64>().is_ok() {
                    Some(Token::Number)
                } else {
                    None
                }
            });
            if let Some(token) = token {
                spans.push((start..ix, token));
            }
        } else {
            ix += 1;
        }
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlights_levels_timestamps_json_and_unicode_without_overlaps() {
        let line = "2026-10-12T01:00:00.123Z ERROR {\"级别\":\"warn\",\"消息\":\"请求\\\"失败\",\"code\":500,\"ok\":false} terror";
        let spans = tokens(line);
        for pair in spans.windows(2) {
            assert!(pair[0].0.end <= pair[1].0.start);
        }
        assert!(
            spans
                .iter()
                .any(|(range, token)| &line[range.clone()] == "ERROR" && *token == Token::Error)
        );
        assert!(
            spans
                .iter()
                .any(|(range, token)| &line[range.clone()] == "\"级别\"" && *token == Token::Key)
        );
        assert!(
            spans.iter().any(
                |(range, token)| &line[range.clone()] == "\"warn\"" && *token == Token::Warning
            )
        );
        assert!(spans.iter().any(|(_, token)| *token == Token::Timestamp));
        assert!(
            !spans
                .iter()
                .any(|(range, _)| &line[range.clone()] == "terror")
        );
    }

    #[test]
    fn latest_buffer_and_display_stay_bounded_for_many_and_huge_lines() {
        let (text, clipped) = bounded((0..20_000).map(|ix| format!("line-{ix}\n")).collect());
        assert!(clipped);
        assert_eq!(text.lines().count(), MAX_LINES);
        assert!(text.starts_with("line-10000\n"));
        let (text, clipped) = bounded("界".repeat(MAX_BYTES));
        assert!(clipped);
        assert!(text.len() <= MAX_BYTES);
        let row = LogLine::new(&text);
        assert_eq!(row.raw, text);
        assert!(row.text.len() <= DISPLAY_BYTES + '…'.len_utf8());
        for (range, _) in row.spans {
            assert!(row.text.get(range).is_some());
        }
    }
}

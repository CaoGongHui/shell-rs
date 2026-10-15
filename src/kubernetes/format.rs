//! Display conversions only; API quantities and editable manifests stay intact.
use chrono::{DateTime, FixedOffset, SecondsFormat};

pub(super) fn memory(quantity: &str) -> String {
    super::metrics::quantity(quantity)
        .map(memory_bytes)
        .unwrap_or_else(|| quantity.to_owned())
}

/// G follows the binary memory scale used by the system monitor (1024³ bytes).
pub(super) fn memory_bytes(bytes: f64) -> String {
    let gib = bytes / 1024_f64.powi(3);
    if gib > 0. && gib < 0.01 {
        "<0.01 G".into()
    } else {
        format!("{gib:.2} G")
    }
}

fn east_eight() -> FixedOffset {
    FixedOffset::east_opt(8 * 60 * 60).expect("UTC+08:00 is a valid offset")
}

pub(super) fn time(value: &str) -> String {
    DateTime::parse_from_rfc3339(value).map_or_else(
        |_| value.to_owned(),
        |time| {
            time.with_timezone(&east_eight())
                .format("%Y-%m-%d %H:%M:%S %:z")
                .to_string()
        },
    )
}

/// Convert only kubectl's timestamp prefix, preserving application log content.
pub(super) fn log_line(line: &str) -> String {
    let end = line.find(char::is_whitespace).unwrap_or(line.len());
    DateTime::parse_from_rfc3339(&line[..end]).map_or_else(
        |_| line.to_owned(),
        |time| {
            format!(
                "{}{}",
                time.with_timezone(&east_eight())
                    .to_rfc3339_opts(SecondsFormat::AutoSi, false),
                &line[end..]
            )
        },
    )
}

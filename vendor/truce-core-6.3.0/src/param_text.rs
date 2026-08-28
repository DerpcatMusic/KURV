//! Host-side parameter display-text parsing.

use truce_params::{ParamInfo, ParamRange, ParamUnit, ParamValueKind};

/// Parse display text, then require the candidate to format back identically.
///
/// This runs only in host text-entry callbacks. Discrete searches are bounded
/// so malformed metadata cannot turn a host query into unbounded work.
pub fn parse_formatted_value(
    info: &ParamInfo,
    text: &str,
    mut format: impl FnMut(f64) -> String,
) -> Option<f64> {
    let text = text.trim();
    let matches = |candidate: f64, format: &mut dyn FnMut(f64) -> String| {
        format(candidate).trim().eq_ignore_ascii_case(text)
    };

    let discrete = match info.kind {
        ParamValueKind::Bool => Some((0_i64, 1_i64)),
        _ => match info.range {
            ParamRange::Discrete { min, max } => Some((min, max)),
            ParamRange::Enum { count } => i64::try_from(count.saturating_sub(1))
                .ok()
                .map(|max| (0, max)),
            _ => None,
        },
    };
    if let Some((min, max)) = discrete
        && max >= min
        && max.saturating_sub(min) <= 4_096
    {
        for value in min..=max {
            let candidate = value as f64;
            if matches(candidate, &mut format) {
                return Some(candidate);
            }
        }
    }

    let candidate = parse_display_number(info, text)?;
    matches(candidate, &mut format).then_some(candidate)
}

fn parse_display_number(info: &ParamInfo, text: &str) -> Option<f64> {
    let lower = text.trim().to_ascii_lowercase();
    match lower.as_str() {
        "off" | "false" | "linear" | "even" | "center" => return Some(0.0),
        "on" | "true" => return Some(1.0),
        "c" if info.unit == ParamUnit::Pan => return Some(0.0),
        _ => {}
    }

    let mut value = lower.split_whitespace().find_map(parse_number_token)?;
    if lower.ends_with('l') && info.unit == ParamUnit::Pan {
        return Some(-value.abs() / 100.0);
    }
    if lower.ends_with('r') && info.unit == ParamUnit::Pan {
        return Some(value.abs() / 100.0);
    }
    if lower.contains("khz") {
        value *= 1_000.0;
    } else if lower.ends_with("ms") && info.unit == ParamUnit::Seconds {
        value /= 1_000.0;
    } else if lower.contains('%') {
        value /= 100.0;
    } else if (lower.contains('°') || lower.ends_with("deg"))
        && info.range.min() >= 0.0
        && info.range.max() <= 1.0
    {
        value /= 360.0;
    }

    if ["slow", "edges", "early"]
        .iter()
        .any(|prefix| lower.starts_with(prefix))
    {
        value = -value.abs();
    }
    Some(value)
}

fn parse_number_token(token: &str) -> Option<f64> {
    token
        .trim_matches(|c: char| !c.is_ascii_digit() && !matches!(c, '+' | '-' | '.' | 'e' | 'E'))
        .parse()
        .ok()
}

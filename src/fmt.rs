use std::time::Duration;

const UNITS: [&str; 7] = ["", "K", "M", "G", "T", "P", "E"];

/// Format a byte count with three significant digits, e.g. `4.21G`.
pub fn human_bytes(n: u64) -> String {
    if n < 1024 {
        return n.to_string();
    }
    let mut value = n as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    let suffix = UNITS[unit];

    // Choose precision based on value, accounting for rounding boundaries.
    // Thresholds are set so rounding never produces four significant digits or >= 1024.
    if value < 9.995 {
        format!("{value:.2}{suffix}")
    } else if value < 99.95 {
        format!("{value:.1}{suffix}")
    } else if value < 1023.5 {
        format!("{value:.0}{suffix}")
    } else {
        // Would round to >= 1024, promote to next unit
        if unit + 1 < UNITS.len() {
            value /= 1024.0;
            unit += 1;
            format!("{value:.2}{}", UNITS[unit])
        } else {
            // At max unit (exabytes), just format as is
            format!("{value:.0}{suffix}")
        }
    }
}

/// Format a transfer rate, e.g. `118MB/s`. `None` and zero render as unknown.
pub fn human_rate(bytes_per_sec: Option<f64>) -> String {
    match bytes_per_sec {
        Some(r) if r > 0.0 && r.is_finite() => format!("{}B/s", human_bytes(r as u64)),
        _ => "--.-KB/s".to_string(),
    }
}

/// Format a duration as `22s`, `12m41s`, or `1h02m41s`.
pub fn human_duration(d: Duration) -> String {
    let total = d.as_secs();
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}h{m:02}m{s:02}s")
    } else if m > 0 {
        format!("{m}m{s:02}s")
    } else {
        format!("{s}s")
    }
}

/// Format an ETA field, including the `eta ` prefix, with an unknown form.
pub fn human_eta(d: Option<Duration>) -> String {
    match d {
        Some(d) => format!("eta {}", human_duration(d)),
        None => "eta --".to_string(),
    }
}

/// Format a file count with thousands separators, e.g. `41,203`.
pub fn human_count(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn bytes_below_one_kib_render_as_plain_numbers() {
        assert_eq!(human_bytes(0), "0");
        assert_eq!(human_bytes(512), "512");
        assert_eq!(human_bytes(1023), "1023");
    }

    #[test]
    fn bytes_use_three_significant_digits() {
        assert_eq!(human_bytes(1024), "1.00K");
        assert_eq!(human_bytes(1024 * 10), "10.0K");
        assert_eq!(human_bytes(1024 * 100), "100K");
    }

    #[test]
    fn bytes_scale_past_thirty_two_bits() {
        // 100 GB — the headline case. Must not truncate.
        assert_eq!(human_bytes(100 * 1024 * 1024 * 1024), "100G");
        assert_eq!(human_bytes(4_519_235_912), "4.21G");
        assert_eq!(human_bytes(10 * 1024_u64.pow(4)), "10.0T");
        assert_eq!(human_bytes(5 * 1024_u64.pow(5)), "5.00P");
    }

    #[test]
    fn max_u64_does_not_panic_or_overflow() {
        let s = human_bytes(u64::MAX);
        assert!(s.ends_with('E'), "expected exabyte unit, got {s}");
    }

    #[test]
    fn rate_renders_per_second_and_has_an_unknown_form() {
        assert_eq!(human_rate(Some(118.0 * 1024.0 * 1024.0)), "118MB/s");
        assert_eq!(human_rate(None), "--.-KB/s");
        assert_eq!(human_rate(Some(0.0)), "--.-KB/s");
    }

    #[test]
    fn durations_render_compactly() {
        assert_eq!(human_duration(Duration::from_secs(0)), "0s");
        assert_eq!(human_duration(Duration::from_secs(22)), "22s");
        assert_eq!(human_duration(Duration::from_secs(761)), "12m41s");
        assert_eq!(human_duration(Duration::from_secs(3761)), "1h02m41s");
    }

    #[test]
    fn eta_has_an_unknown_form() {
        assert_eq!(human_eta(Some(Duration::from_secs(22))), "eta 22s");
        assert_eq!(human_eta(None), "eta --");
    }

    #[test]
    fn counts_are_thousands_separated() {
        assert_eq!(human_count(0), "0");
        assert_eq!(human_count(999), "999");
        assert_eq!(human_count(41_203), "41,203");
        assert_eq!(human_count(1_000_000), "1,000,000");
    }

    #[test]
    fn boundary_rounding_maintains_three_significant_digits() {
        // Rounding must not produce four significant digits
        assert_eq!(human_bytes(10239), "10.0K");
        assert_eq!(human_bytes(104847051), "100M");
        assert_eq!(human_bytes(107374182399), "100G");

        // Unit promotion when rounding would exceed 1023
        assert_eq!(human_bytes(1048575), "1.00M");
        assert_eq!(human_bytes(1073741823), "1.00G");
    }
}

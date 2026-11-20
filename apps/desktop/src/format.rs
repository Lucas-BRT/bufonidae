//! Small display helpers.

/// `1_500_000` -> `1.5 MB` (decimal units, as file managers show them).
pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    #[allow(clippy::cast_precision_loss)]
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// `1` -> `1 file`, `2` -> `2 files`.
pub fn files(n: usize) -> String {
    format!("{n} file{}", if n == 1 { "" } else { "s" })
}

/// Parses a size in decimal gigabytes typed by the user (`1`, `0.5`, `2,5`)
/// into bytes. 0 is allowed (it means "always ask").
pub fn parse_gb(text: &str) -> Result<u64, String> {
    let normalized = text.trim().replace(',', ".");
    let gb: f64 = normalized
        .parse()
        .map_err(|_| "Enter a number of gigabytes, like 1 or 0.5.".to_owned())?;
    if !gb.is_finite() || gb < 0.0 {
        return Err("The limit cannot be negative.".to_owned());
    }
    if gb > 1_000_000.0 {
        return Err("That is more than any account can hold.".to_owned());
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok((gb * 1e9).round() as u64)
}

/// Bytes as the decimal-gigabyte text shown in the settings field.
pub fn gb_text(bytes: u64) -> String {
    #[allow(clippy::cast_precision_loss)]
    let gb = bytes as f64 / 1e9;
    let text = format!("{gb:.3}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// `90` seconds ago -> `1 min ago`; coarse on purpose, the exact time is in a tooltip.
pub fn ago(now: i64, then: i64) -> String {
    let secs = (now - then).max(0);
    match secs {
        0..=59 => "just now".to_owned(),
        60..=3599 => format!("{} min ago", secs / 60),
        3600..=86_399 => format!("{} h ago", secs / 3600),
        _ => format!("{} d ago", secs / 86_400),
    }
}

/// Seconds since the Unix epoch as `2026-11-18 14:03 UTC`. UTC is stated, not
/// guessed: the standard library has no local time zone.
pub fn utc(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rest = secs.rem_euclid(86_400);
    // Civil-from-days (proleptic Gregorian), after Howard Hinnant.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        rest / 3600,
        rest % 3600 / 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_sizes() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(999), "999 B");
        assert_eq!(bytes(1_500), "1.5 KB");
        assert_eq!(bytes(2_000_000_000), "2.0 GB");
        assert_eq!(bytes(u64::MAX), "18446744.1 TB");
    }

    #[test]
    fn pluralizes() {
        assert_eq!(files(1), "1 file");
        assert_eq!(files(0), "0 files");
    }

    #[test]
    fn parses_and_prints_gigabytes() {
        assert_eq!(parse_gb("1"), Ok(1_000_000_000));
        assert_eq!(parse_gb(" 0.5 "), Ok(500_000_000));
        assert_eq!(parse_gb("2,5"), Ok(2_500_000_000));
        assert_eq!(parse_gb("0"), Ok(0));
        for bad in ["", "abc", "-1", "NaN", "inf", "2000000"] {
            assert!(parse_gb(bad).is_err(), "{bad}");
        }
        assert_eq!(gb_text(1_000_000_000), "1");
        assert_eq!(gb_text(2_500_000_000), "2.5");
        assert_eq!(gb_text(0), "0");
        assert_eq!(parse_gb(&gb_text(750_000_000)), Ok(750_000_000));
    }

    #[test]
    fn times_read_plainly() {
        assert_eq!(ago(100, 100), "just now");
        assert_eq!(ago(1000, 100), "15 min ago");
        assert_eq!(ago(10_000, 100), "2 h ago");
        assert_eq!(ago(300_000, 0), "3 d ago");
        assert_eq!(utc(0), "1970-01-01 00:00 UTC");
        assert_eq!(utc(1_700_000_000), "2023-11-14 22:13 UTC");
        assert_eq!(utc(951_782_400), "2000-02-29 00:00 UTC");
    }
}

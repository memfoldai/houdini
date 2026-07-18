pub fn parse_rfc3339_ms(s: &str) -> Option<i64> {
    let s = s.trim();
    let (date, rest) = s.split_once(['T', 't', ' '])?;

    let (time, offset_min) = if let Some(t) = rest.strip_suffix(['Z', 'z']) {
        (t, 0i64)
    } else if let Some(pos) = rest.rfind(['+', '-']) {
        let (t, zone) = rest.split_at(pos);
        (t, parse_offset_min(zone)?)
    } else {
        (rest, 0)
    };

    let mut dparts = date.split('-');
    let y: i64 = dparts.next()?.parse().ok()?;
    let mo: i64 = dparts.next()?.parse().ok()?;
    let d: i64 = dparts.next()?.parse().ok()?;
    if dparts.next().is_some() || !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }

    let mut tparts = time.split(':');
    let h: i64 = tparts.next()?.parse().ok()?;
    let mi: i64 = tparts.next()?.parse().ok()?;
    let (sec, frac_ms) = match tparts.next() {
        Some(sec_str) => parse_seconds_ms(sec_str)?,
        None => (0, 0),
    };
    if tparts.next().is_some() || h > 23 || mi > 59 || sec > 60 {
        return None;
    }

    let days = days_from_civil(y, mo, d);
    let ms = days * 86_400_000 + (h * 3600 + mi * 60 + sec) * 1000 + frac_ms - offset_min * 60_000;
    Some(ms)
}

pub fn ymd_utc(unix_ms: i64) -> String {
    let days = unix_ms.div_euclid(86_400_000);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    format!("{year:04}-{m:02}-{d:02}")
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn parse_seconds_ms(s: &str) -> Option<(i64, i64)> {
    let (whole, frac) = match s.split_once('.') {
        Some((w, f)) => (w, f),
        None => (s, ""),
    };
    let sec: i64 = whole.parse().ok()?;
    if frac.is_empty() {
        return Some((sec, 0));
    }

    let mut ms = 0i64;
    for i in 0..3 {
        ms = ms * 10
            + frac
                .as_bytes()
                .get(i)
                .map(|b| (b - b'0') as i64)
                .unwrap_or(0);
    }
    if !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((sec, ms))
}

fn parse_offset_min(zone: &str) -> Option<i64> {
    let (sign, rest) = match zone.as_bytes().first()? {
        b'+' => (1, &zone[1..]),
        b'-' => (-1, &zone[1..]),
        _ => return None,
    };
    let rest = rest.replace(':', "");
    if rest.len() != 4 || !rest.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let h: i64 = rest[..2].parse().ok()?;
    let m: i64 = rest[2..].parse().ok()?;
    Some(sign * (h * 60 + m))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_known_instants() {
        assert_eq!(ymd_utc(0), "1970-01-01");
        assert_eq!(parse_rfc3339_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339_ms("1970-01-01T00:00:00.000Z"), Some(0));
    }

    #[test]
    fn parses_transcript_timestamps() {
        let ms = parse_rfc3339_ms("2026-07-02T07:50:50.556Z").unwrap();
        assert_eq!(ymd_utc(ms), "2026-07-02");

        assert!(parse_rfc3339_ms("2026-07-01T09:32:06Z").is_some());
    }

    #[test]
    fn applies_zone_offset() {
        let z = parse_rfc3339_ms("2026-07-02T07:50:50Z").unwrap();
        let plus2 = parse_rfc3339_ms("2026-07-02T09:50:50+02:00").unwrap();
        assert_eq!(z, plus2, "same instant expressed in +02:00");
        let minus5 = parse_rfc3339_ms("2026-07-02T02:50:50-05:00").unwrap();
        assert_eq!(z, minus5);
    }

    #[test]
    fn rejects_malformed() {
        assert_eq!(parse_rfc3339_ms("not-a-time"), None);
        assert_eq!(parse_rfc3339_ms("2026-13-01T00:00:00Z"), None);
        assert_eq!(parse_rfc3339_ms("2026-07-02T25:00:00Z"), None);
    }

    #[test]
    fn fractional_padding_is_milliseconds() {
        let a = parse_rfc3339_ms("2026-07-02T00:00:00.5Z").unwrap();
        let b = parse_rfc3339_ms("2026-07-02T00:00:00.500Z").unwrap();
        assert_eq!(a, b);
        assert_eq!(a % 1000, 500);
    }
}

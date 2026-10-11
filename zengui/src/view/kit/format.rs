//! Numbers as words: bytes, rates, plurals.

/// Human-readable byte count.
pub fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = n as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[unit])
    }
}

/// `n` of something, pluralised. "1 keys" reads as a bug in the tool, and so
/// do "2 watchs" and "2 addresss": a noun ending in a sibilant takes `-es`,
/// and one ending in a consonant and `y` takes `-ies` (#614, FL6b, whose
/// strip counts addresses and watches).
pub fn plural(n: usize, singular: &str) -> String {
    if n == 1 {
        return format!("{n} {singular}");
    }
    let sibilant = ["s", "x", "z", "ch", "sh"]
        .iter()
        .any(|end| singular.ends_with(end));
    let consonant_y = singular.ends_with('y')
        && !singular
            .chars()
            .rev()
            .nth(1)
            .is_some_and(|c| "aeiou".contains(c));
    if sibilant {
        format!("{n} {singular}es")
    } else if consonant_y {
        format!("{n} {}ies", &singular[..singular.len() - 1])
    } else {
        format!("{n} {singular}s")
    }
}

/// Human-readable rate.
pub fn human_rate(hz: f64) -> String {
    if hz <= 0.0 {
        "—".to_string()
    } else if hz < 1.0 {
        format!("{hz:.2}/s")
    } else if hz < 1000.0 {
        format!("{hz:.1}/s")
    } else {
        format!("{:.1}k/s", hz / 1000.0)
    }
}

/// An age as one short word (#538): "now" under three seconds — a key seen
/// within the last few monitor ticks — then "12s", "4m", "2h".
pub fn age_word(secs: f32) -> String {
    let s = secs.max(0.0);
    if s < 3.0 {
        "now".to_string()
    } else if s < 60.0 {
        format!("{}s", s as u32)
    } else if s < 3600.0 {
        format!("{}m", (s / 60.0) as u32)
    } else {
        format!("{}h", (s / 3600.0) as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::plural;

    #[test]
    fn plurals_read_as_english() {
        assert_eq!(plural(1, "watch"), "1 watch");
        assert_eq!(plural(2, "watch"), "2 watches");
        assert_eq!(plural(0, "address"), "0 addresses");
        assert_eq!(plural(3, "reply"), "3 replies");
        assert_eq!(plural(2, "key"), "2 keys");
        assert_eq!(plural(2, "revision"), "2 revisions");
    }
}

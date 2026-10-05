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

/// `n` of something, pluralised. "1 keys" reads as a bug in the tool.
pub fn plural(n: usize, singular: &str) -> String {
    if n == 1 {
        format!("{n} {singular}")
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

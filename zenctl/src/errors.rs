//! Rendering an error for a person (#240).
//!
//! zenoh's error types append their own source location — ` at <file>:<line>.`
//! — and the file is one inside the **cargo registry of the machine that built
//! the binary**. Passed through verbatim, the longest part of a one-line error
//! is a path that exists on nobody's computer:
//!
//! ```text
//! Invalid Key Expr `v1/**a`: `*` may only be preceded by `/` or `$` at
//! /srv/dev/cargo/registry/src/index.crates.io-…/key_expr/borrowed.rs:777.
//! ```
//!
//! Trimming it is `zenctl`'s job under this workspace's division of labour:
//! the engine owns bus-or-registry input through to a `report::*` value, and
//! zenctl owns turning a value into characters. It is also what makes the
//! sentences an operator reads when something is wrong *pinnable* — a corpus
//! that has to wildcard them (#201) has stopped guarding exactly those lines.
//!
//! Deliberately **not** applied to `tracing` output: there the location is a
//! developer surface and it is the point.

/// Strip zenoh's trailing ` at <path>.rs:<line>.` source locations.
///
/// Conservative on purpose: a tail is only removed when it looks like a Rust
/// source location — a slash-bearing path ending `.rs`, a colon, and digits.
/// An error legitimately ending in the word "at" keeps it, and so does a
/// message that names a *user's* file.
pub fn without_source_locations(text: &str) -> String {
    text.lines().map(trim_line).collect::<Vec<_>>().join("\n")
}

fn trim_line(line: &str) -> String {
    let mut out = line;
    while let Some(i) = out.rfind(" at ") {
        if is_source_location(&out[i + 4..]) {
            out = out[..i].trim_end();
        } else {
            break;
        }
    }
    out.to_string()
}

fn is_source_location(tail: &str) -> bool {
    let tail = tail.strip_suffix('.').unwrap_or(tail);
    let Some((path, line)) = tail.rsplit_once(':') else {
        return false;
    };
    path.contains('/')
        && path.ends_with(".rs")
        && !line.is_empty()
        && line.bytes().all(|b| b.is_ascii_digit())
}

/// An `anyhow` chain, rendered the way `anyhow`'s own `Termination` renders
/// it — `Error: …` then an indented `Caused by:` list — with every source
/// location trimmed.
///
/// Reimplemented rather than post-processed because the default rendering
/// only happens when `main` *returns* the error, and by then there is nothing
/// left to filter.
pub fn render(err: &anyhow::Error) -> String {
    let mut out = format!("Error: {}", without_source_locations(&err.to_string()));
    let causes: Vec<String> = err
        .chain()
        .skip(1)
        .map(|c| without_source_locations(&c.to_string()))
        .collect();
    match causes.len() {
        0 => {}
        1 => out.push_str(&format!("\n\nCaused by:\n    {}", causes[0])),
        _ => {
            out.push_str("\n\nCaused by:");
            for (i, c) in causes.iter().enumerate() {
                out.push_str(&format!("\n    {i}: {c}"));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The line #240 was filed over, and the shape of every one like it.
    #[test]
    fn a_zenoh_source_location_is_trimmed_and_the_message_is_not() {
        assert_eq!(
            without_source_locations(
                "Invalid Key Expr `v1/**a`: `*` may only be preceded by `/` or `$` at \
                 /srv/dev/cargo/registry/src/index.crates.io-1949cf8c6b5b557f/\
                 zenoh-keyexpr-1.9.0/src/key_expr/borrowed.rs:777."
            ),
            "Invalid Key Expr `v1/**a`: `*` may only be preceded by `/` or `$`"
        );
        // Nested locations: zenoh's link errors carry two.
        assert_eq!(
            without_source_locations(
                "Can not create a link at /a/b/tcp.rs:96. at /c/d/unicast.rs:280."
            ),
            "Can not create a link"
        );
    }

    /// A trimmer that eats real prose is worse than the paths it removes.
    #[test]
    fn a_message_that_merely_ends_in_at_keeps_its_words() {
        for kept in [
            "no reply from the origin we looked at",
            "registry dir not found: ./registry",
            "the file at ./config.json5 sets a namespace",
            "failed at 12:00",
        ] {
            assert_eq!(without_source_locations(kept), kept, "trimmed real prose");
        }
    }

    /// Multi-line errors are trimmed per line, so an anyhow chain keeps its
    /// shape while losing its paths.
    #[test]
    fn each_line_of_a_chain_is_trimmed_on_its_own() {
        assert_eq!(
            without_source_locations("first at /x/a.rs:1.\nsecond at /y/b.rs:2."),
            "first\nsecond"
        );
    }
}

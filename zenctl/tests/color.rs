//! Colour, and the rule it lives under (#200).
//!
//! > Colour may only re-encode a distinction the plain text already makes.
//! > Stripping every escape sequence must leave output with identical
//! > information content.
//!
//! Three claims, and the third is the important one:
//!
//! 1. `NO_COLOR` and `--color never` agree, byte for byte;
//! 2. a machine-readable stream carries no escape, whatever `--color` says;
//! 3. **stripping every escape from a coloured rendering yields the plain
//!    rendering** — which is the rule itself, checked rather than promised.
//!
//! The fourth acceptance #200 asks for — "the plain-text rendering of every
//! verdict enum is unchanged from before this issue" — is not a test here. It
//! is `tests/render.rs`: those snapshots were captured before colour existed
//! and they are untouched by it, so a passing run *is* that assertion.

use zenctl::render::{ColorChoice, Format, Render, Width, to_string_with};
use zenkey_report_fixtures as fx;

const W: Width = Width::Unbounded;

fn render<R: Render>(r: &R, format: Format, color: ColorChoice) -> String {
    to_string_with(r, format, W, color).expect("render").0
}

/// The rule, checked. Not "colour is tasteful" — *stripping it changes
/// nothing*, which is what makes it safe to pipe.
#[test]
fn stripping_every_escape_leaves_the_plain_rendering_byte_for_byte() {
    let doctor = fx::doctor_report();
    let topics = fx::topic_list();
    for (name, coloured, plain) in [
        (
            "doctor",
            render(&doctor, Format::Table, ColorChoice::Always),
            render(&doctor, Format::Table, ColorChoice::Never),
        ),
        (
            "topic-list",
            render(&topics, Format::Table, ColorChoice::Always),
            render(&topics, Format::Table, ColorChoice::Never),
        ),
    ] {
        assert_ne!(coloured, plain, "{name}: nothing was coloured at all");
        assert_eq!(
            anstream::adapter::strip_str(&coloured).to_string(),
            plain,
            "{name}: colour carried something the plain text does not"
        );
    }
}

/// Alignment survives, because the padding is written outside the escapes.
///
/// Colour inside the padding is the classic table-alignment bug: every line
/// still *looks* padded to the same width while carrying a different number of
/// visible characters.
#[test]
fn colour_does_not_shift_a_column() {
    let coloured = render(&fx::topic_list(), Format::Table, ColorChoice::Always);
    let plain = render(&fx::topic_list(), Format::Table, ColorChoice::Never);
    let widths = |s: &str| -> Vec<usize> {
        anstream::adapter::strip_str(s)
            .to_string()
            .lines()
            .map(|l| l.chars().count())
            .collect()
    };
    assert_eq!(widths(&coloured), widths(&plain));
}

/// An escape in a machine-readable stream is a bug no downstream parser should
/// have to defend against — and here it is not a case that is handled but one
/// with no spelling: `Styling` is built inside the table arm and reaches the
/// writer only through `Table`, and `Render::rows` returns `serde_json::Value`.
#[test]
fn a_machine_readable_stream_never_carries_an_escape() {
    for format in [Format::Json, Format::Ndjson] {
        for color in [ColorChoice::Always, ColorChoice::Auto, ColorChoice::Never] {
            let out = render(&fx::doctor_report(), format, color);
            assert!(
                !out.contains('\u{1b}'),
                "{format:?} with {color:?} carried an escape"
            );
            // And it is still parseable, which is the reason that matters.
            for line in out.lines() {
                serde_json::from_str::<serde_json::Value>(line)
                    .or_else(|_| serde_json::from_str::<serde_json::Value>(&out))
                    .expect("a machine format stays machine-readable");
            }
        }
    }
}

/// `--color never` is what `NO_COLOR` means, so they must not be two ways of
/// getting almost the same thing.
#[test]
fn no_color_and_color_never_agree() {
    // `NO_COLOR` is read by `ColorChoice::Auto`; setting it is what a user
    // does once for every tool on the machine, and this asserts zenctl is one
    // of them. Scoped tightly: the process is shared with the other tests in
    // this binary, and `Auto` is the only choice that reads it.
    let plain = render(&fx::doctor_report(), Format::Table, ColorChoice::Never);
    // SAFETY: single-threaded within this test, and removed before returning.
    unsafe { std::env::set_var("NO_COLOR", "1") };
    let auto = render(&fx::doctor_report(), Format::Table, ColorChoice::Auto);
    unsafe { std::env::remove_var("NO_COLOR") };
    assert_eq!(auto, plain, "NO_COLOR and --color never are the same thing");
}

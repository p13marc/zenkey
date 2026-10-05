//! Payload preview syntax (#538): where the keys, strings and literals are
//! in a one-line JSON or CBOR-diagnostic preview.
//!
//! Pure and lexical — no parse, no allocation per character — and run **once
//! per sample at ingest** ([`crate::echo::EchoLine::render`]), never on a
//! redraw: the #345 rule that a frame touches no payload holds for colour
//! too. Tolerant by construction: a preview is truncated at
//! `PREVIEW_CHARS`, so an unterminated string or a dangling brace is the
//! ordinary case, not an error. Anything that does not open like a JSON
//! document (`{` or `[`) gets no spans at all and renders as plain text —
//! colouring prose by guesswork would be inventing structure.

use crate::view::theme::SyntaxRole;

/// One coloured run: a byte range of the preview and what it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyntaxSpan {
    pub start: u32,
    pub end: u32,
    pub role: SyntaxRole,
}

/// The spans of `preview`, covering it end to end, or none at all.
pub fn spans(preview: &str) -> Box<[SyntaxSpan]> {
    let bytes = preview.as_bytes();
    let first = bytes.iter().position(|b| !b.is_ascii_whitespace());
    if !matches!(first.map(|i| bytes[i]), Some(b'{' | b'[')) {
        return Box::default();
    }
    let mut out: Vec<SyntaxSpan> = Vec::new();
    let mut push = |start: usize, end: usize, role: SyntaxRole| {
        if start >= end {
            return;
        }
        // Adjacent runs of one role merge, so a `}, {` is one span.
        if let Some(last) = out.last_mut()
            && last.role == role
            && last.end as usize == start
        {
            last.end = end as u32;
            return;
        }
        out.push(SyntaxSpan {
            start: start as u32,
            end: end as u32,
            role,
        });
    };
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'"' => {
                let start = i;
                i += 1;
                while i < bytes.len() {
                    match bytes[i] {
                        b'\\' => i += 2,
                        b'"' => {
                            i += 1;
                            break;
                        }
                        _ => i += 1,
                    }
                }
                let end = i.min(bytes.len());
                // A key is a string whose next non-blank byte is `:`.
                let next = bytes[end..].iter().find(|b| !b.is_ascii_whitespace());
                let role = if next == Some(&b':') {
                    SyntaxRole::Key
                } else {
                    SyntaxRole::String
                };
                push(start, end, role);
            }
            b'{' | b'}' | b'[' | b']' | b',' | b':' => {
                push(i, i + 1, SyntaxRole::Punct);
                i += 1;
            }
            _ if b.is_ascii_whitespace() => {
                // Blank runs join whatever came before, so they need no span
                // of their own — but the cover must stay end to end.
                let start = i;
                while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                push(start, i, SyntaxRole::Punct);
            }
            _ => {
                // A literal: a number, true/false/null, a CBOR diagnostic
                // form (h'…', 1(…)) — anything up to the next delimiter.
                let start = i;
                while i < bytes.len()
                    && !matches!(bytes[i], b'{' | b'}' | b'[' | b']' | b',' | b':' | b'"')
                    && !bytes[i].is_ascii_whitespace()
                {
                    i += 1;
                }
                push(start, i, SyntaxRole::Literal);
            }
        }
    }
    // Never split a char: every boundary above is an ASCII byte, so a
    // multi-byte char sits wholly inside one span — except a string cut by
    // the escape skip at the very end, which `min` clamps to the length.
    out.into_boxed_slice()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roles(s: &str) -> Vec<(&str, SyntaxRole)> {
        spans(s)
            .iter()
            .map(|sp| (&s[sp.start as usize..sp.end as usize], sp.role))
            .collect()
    }

    #[test]
    fn keys_strings_and_literals_are_told_apart() {
        let r = roles(r#"{"value":61.80,"unit":"%","ok":true}"#);
        assert!(r.contains(&(r#""value""#, SyntaxRole::Key)));
        assert!(r.contains(&(r#""%""#, SyntaxRole::String)));
        assert!(r.contains(&("61.80", SyntaxRole::Literal)));
        assert!(r.contains(&("true", SyntaxRole::Literal)));
    }

    /// The spans tile the preview exactly: the plain text a test finds and
    /// the coloured text a user reads are the same characters.
    #[test]
    fn the_spans_cover_the_preview_end_to_end() {
        for s in [
            r#"{"a": [1, 2, {"b": null}], "c": "x\"y"}"#,
            r#"{"truncated": "never clo"#,
            "[1, h'0a0b', 2(\"tag\")]",
            r#"{"é": "ünïcode ✓"}"#,
        ] {
            let sp = spans(s);
            assert_eq!(sp.first().map(|x| x.start), Some(0), "{s}");
            assert_eq!(sp.last().map(|x| x.end as usize), Some(s.len()), "{s}");
            for w in sp.windows(2) {
                assert_eq!(w[0].end, w[1].start, "{s}");
            }
            for x in sp.iter() {
                assert!(s.is_char_boundary(x.start as usize) && s.is_char_boundary(x.end as usize));
            }
        }
    }

    /// Prose is not coloured by guesswork: no document, no spans.
    #[test]
    fn text_that_is_not_a_document_gets_no_spans() {
        for s in [
            "hello #3",
            "<delete>",
            "<12 bytes — too large to preview>",
            "42",
            "",
        ] {
            assert!(spans(s).is_empty(), "{s}");
        }
    }
}

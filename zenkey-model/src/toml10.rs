//! TOML 1.0 only (spec `core.md` §9.1): a contract MUST NOT need TOML 1.1,
//! because a 1.0 reader (Python's `tomllib`, for one) refuses it. The `toml`
//! crate reads 1.1, so the syntax that only 1.1 has is found here, on
//! `toml_parser`'s event stream, and reported as E000. So is an integer
//! outside TOML 1.0's 64-bit range, which readers disagree on: the `toml`
//! crate reads up to 2^64−1, `tomllib` any size.

use toml_parser::decoder::Encoding;
use toml_parser::parser::{EventReceiver, parse_document};
use toml_parser::{ErrorSink, Source, Span};

/// The first piece of TOML 1.1-only syntax, or the first integer beyond 64
/// bits, in `text`: its 1-based line and what it is. `text` is expected to
/// parse; a parse error is the parser's to report, and is ignored here.
pub(crate) fn first_beyond_toml10(text: &str) -> Option<(usize, &'static str)> {
    let tokens = Source::new(text).lex().into_vec();
    let mut scan = Scan {
        text,
        open: Vec::new(),
        after_sep: false,
        found: None,
    };
    parse_document(&tokens, &mut scan, &mut ());
    scan.found
        .map(|(at, what)| (text[..at].matches('\n').count() + 1, what))
}

struct Scan<'t> {
    text: &'t str,
    /// The open containers, innermost last: `true` for an inline table.
    open: Vec<bool>,
    /// A `,` was the last thing seen inside the innermost inline table.
    after_sep: bool,
    found: Option<(usize, &'static str)>,
}

impl Scan<'_> {
    fn in_inline_table(&self) -> bool {
        self.open.last() == Some(&true)
    }

    fn found(&mut self, span: Span, what: &'static str) {
        self.found.get_or_insert((span.start(), what));
    }
}

impl EventReceiver for Scan<'_> {
    fn inline_table_open(&mut self, _span: Span, _error: &mut dyn ErrorSink) -> bool {
        self.open.push(true);
        self.after_sep = false;
        true
    }

    fn inline_table_close(&mut self, span: Span, _error: &mut dyn ErrorSink) {
        if self.after_sep {
            self.found(span, "a trailing comma in an inline table");
        }
        self.open.pop();
        self.after_sep = false;
    }

    fn array_open(&mut self, _span: Span, _error: &mut dyn ErrorSink) -> bool {
        self.open.push(false);
        self.after_sep = false;
        true
    }

    fn array_close(&mut self, _span: Span, _error: &mut dyn ErrorSink) {
        self.open.pop();
        self.after_sep = false;
    }

    fn simple_key(&mut self, _span: Span, _kind: Option<Encoding>, _error: &mut dyn ErrorSink) {
        self.after_sep = false;
    }

    fn value_sep(&mut self, _span: Span, _error: &mut dyn ErrorSink) {
        self.after_sep = self.in_inline_table();
    }

    fn newline(&mut self, span: Span, _error: &mut dyn ErrorSink) {
        if self.in_inline_table() {
            self.found(span, "a newline inside an inline table");
        }
    }

    fn comment(&mut self, span: Span, _error: &mut dyn ErrorSink) {
        if self.in_inline_table() {
            self.found(span, "a comment inside an inline table");
        }
    }

    fn scalar(&mut self, span: Span, kind: Option<Encoding>, _error: &mut dyn ErrorSink) {
        self.after_sep = false;
        let raw = &self.text[span.start()..span.end()];
        match kind {
            Some(Encoding::BasicString | Encoding::MlBasicString) => {
                let mut bytes = raw.bytes();
                while let Some(b) = bytes.next() {
                    if b == b'\\' {
                        match bytes.next() {
                            Some(b'e') => self.found(span, "the `\\e` escape"),
                            Some(b'x') => self.found(span, "the `\\xHH` escape"),
                            _ => {}
                        }
                    }
                }
            }
            Some(_) => {}
            // A bare value: only a time has a `:`, and TOML 1.0 requires
            // its seconds (`HH:MM:SS`).
            None => {
                if let Some(colon) = raw.find(':')
                    && raw.as_bytes().get(colon + 3) != Some(&b':')
                {
                    self.found(span, "a time without seconds");
                }
                if integer_beyond_64_bits(raw) {
                    self.found(span, "an integer outside TOML 1.0's 64-bit range");
                }
            }
        }
    }
}

/// Whether a bare value is a TOML integer that a 64-bit signed integer
/// cannot hold. TOML 1.0 bounds integers to −2^63..2^63−1 and requires an
/// error beyond; the `toml` crate reads up to 2^64−1, and Python's
/// `tomllib` any size, so the bound is checked here (spec §9.1).
fn integer_beyond_64_bits(raw: &str) -> bool {
    let digits: String = raw.chars().filter(|c| *c != '_').collect();
    let (negative, body) = match digits.as_bytes().first() {
        Some(b'-') => (true, &digits[1..]),
        Some(b'+') => (false, &digits[1..]),
        _ => (false, digits.as_str()),
    };
    let (radix, body) = match body.get(..2) {
        Some("0x") => (16, &body[2..]),
        Some("0o") => (8, &body[2..]),
        Some("0b") => (2, &body[2..]),
        _ => (10, body),
    };
    if body.is_empty() || !body.chars().all(|c| c.is_digit(radix)) {
        return false;
    }
    match u128::from_str_radix(body, radix) {
        Ok(n) if negative => n > 1 << 63,
        Ok(n) => n > (1 << 63) - 1,
        Err(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::first_beyond_toml10;

    #[test]
    fn toml11_only_syntax_is_found() {
        for (text, what) in [
            ("t = {\n  a = 1 }\n", "a newline inside an inline table"),
            ("t = { a = 1, }\n", "a trailing comma in an inline table"),
            (
                "t = { a = 1, b = { c = 2, } }\n",
                "a trailing comma in an inline table",
            ),
            ("s = \"a\\eb\"\n", "the `\\e` escape"),
            ("s = \"\"\"a\\x41\"\"\"\n", "the `\\xHH` escape"),
            ("t = 07:32\n", "a time without seconds"),
            ("t = 1979-05-27T07:32Z\n", "a time without seconds"),
            ("t = 1979-05-27 07:32\n", "a time without seconds"),
        ] {
            assert_eq!(
                first_beyond_toml10(text).map(|(_, w)| w),
                Some(what),
                "{text:?}"
            );
        }
        assert_eq!(
            first_beyond_toml10("a = 1\n\nb = { c = 1, }\n"),
            Some((3, "a trailing comma in an inline table"))
        );
    }

    #[test]
    fn toml10_passes() {
        for text in [
            "t = { a = [\n  1,\n  2,\n] }\n",
            "a = [1, 2,]\n",
            "t = { a = [1, 2,] }\n",
            "s = \"a\\\\eb\\u00e9\\t\"\n",
            "s = 'a\\eb\\x41'\n",
            "s = '''a\\e'''\n",
            "s = \"\"\"a \\\n  b\"\"\"\n",
            "t = 07:32:00\n",
            "t = 1979-05-27T07:32:00+07:00\n",
            "t = 1979-05-27 07:32:00.5\n",
            "d = 1979-05-27\n",
            "t = { a = 1 } # a comment after it\n",
            "[x]\n# comment\nk = \"v\" # trailing\n",
            "i = 9223372036854775807\n",
            "i = -9_223_372_036_854_775_808\n",
            "i = 0x7fffffffffffffff\n",
            "f = 1e300\n",
            "f = nan\n",
        ] {
            assert_eq!(first_beyond_toml10(text), None, "{text:?}");
        }
    }

    #[test]
    fn integers_beyond_64_bits_are_found() {
        for text in [
            "i = 9223372036854775808\n",
            "i = +9_223_372_036_854_775_808\n",
            "i = -9223372036854775809\n",
            "i = 0x8000000000000000\n",
            "i = 0o1000000000000000000000\n",
            "t = { a = [1, 18446744073709551616] }\n",
            "i = 340282366920938463463374607431768211456\n",
        ] {
            assert_eq!(
                first_beyond_toml10(text).map(|(_, w)| w),
                Some("an integer outside TOML 1.0's 64-bit range"),
                "{text:?}"
            );
        }
    }
}

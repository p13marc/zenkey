//! A sample's bytes as its data item (§2.6, text 0.2): strict JSON, and
//! CBOR with its bignums, each into a [`Datum`] whose integers are exact.
//!
//! | §2.6 | Here |
//! |---|---|
//! | a JSON sample is strict JSON (F-143): UTF-8, RFC 8259's grammar, nothing after the value but whitespace, no duplicate member at any depth, no lone surrogate escape, every number finite | [`json`]: anything else is `None`, the sample `undecodable` |
//! | an integer's text is read exactly | `Datum::Int` within i128; beyond it, within the double's finite range, `Datum::BigInt` (its nearest double), which no range of §2.6 admits |
//! | a float's text is the nearest double (round to nearest, ties to even) | `str::parse::<f64>`, correctly rounded; an infinite one is not strict JSON |
//! | a CBOR integer: major types 0 and 1, or a bignum, tag 2 or 3 around a byte string, of any length, definite or not (F-144) | [`cbor`]: `Datum::Int`, or `Datum::BigInt` beyond i128 |
//! | every other CBOR tag, and a tag 2 or 3 around anything but a byte string, is read as its content | [`cbor`] |
//! | a CBOR map is read as core §5.2 reads the envelope's: text keys, no duplicate; `undefined` is `null` | [`cbor`]: anything else is `None` |
//! | a CBOR float keeps NaN and the infinities | `Datum::Float`, which reads `not_finite` |
//!
//! Both readers stop at a depth of [`MAX_DEPTH`]: deeper is `None`. No
//! Point is anywhere near it.

use std::collections::BTreeMap;

use super::Datum;

/// How deep an array or an object (a map) may nest.
pub const MAX_DEPTH: usize = 256;

/// One JSON text as a strict JSON data item, or `None` when the bytes are
/// not one.
#[must_use]
pub fn json(bytes: &[u8]) -> Option<Datum> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut p = Json {
        s: text.as_bytes(),
        text,
        at: 0,
    };
    p.ws();
    let v = p.value(0)?;
    p.ws();
    (p.at == p.s.len()).then_some(v)
}

struct Json<'a> {
    s: &'a [u8],
    text: &'a str,
    at: usize,
}

impl Json<'_> {
    fn ws(&mut self) {
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.s.get(self.at) {
            self.at += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.at).copied()
    }

    fn eat(&mut self, b: u8) -> Option<()> {
        (self.peek()? == b).then(|| self.at += 1)
    }

    fn literal(&mut self, word: &str, v: Datum) -> Option<Datum> {
        self.s[self.at..]
            .starts_with(word.as_bytes())
            .then(|| self.at += word.len())?;
        Some(v)
    }

    fn value(&mut self, depth: usize) -> Option<Datum> {
        match self.peek()? {
            b'{' => self.object(depth + 1),
            b'[' => self.array(depth + 1),
            b'"' => self.string().map(Datum::Text),
            b't' => self.literal("true", Datum::Bool(true)),
            b'f' => self.literal("false", Datum::Bool(false)),
            b'n' => self.literal("null", Datum::Null),
            b'-' | b'0'..=b'9' => self.number(),
            _ => None,
        }
    }

    fn object(&mut self, depth: usize) -> Option<Datum> {
        if depth > MAX_DEPTH {
            return None;
        }
        self.eat(b'{')?;
        let mut m = BTreeMap::new();
        self.ws();
        if self.eat(b'}').is_some() {
            return Some(Datum::Map(m));
        }
        loop {
            self.ws();
            let k = self.string()?;
            self.ws();
            self.eat(b':')?;
            self.ws();
            let v = self.value(depth)?;
            if m.insert(k, v).is_some() {
                return None;
            }
            self.ws();
            if self.eat(b',').is_none() {
                self.eat(b'}')?;
                return Some(Datum::Map(m));
            }
        }
    }

    fn array(&mut self, depth: usize) -> Option<Datum> {
        if depth > MAX_DEPTH {
            return None;
        }
        self.eat(b'[')?;
        let mut a = Vec::new();
        self.ws();
        if self.eat(b']').is_some() {
            return Some(Datum::Array(a));
        }
        loop {
            self.ws();
            a.push(self.value(depth)?);
            self.ws();
            if self.eat(b',').is_none() {
                self.eat(b']')?;
                return Some(Datum::Array(a));
            }
        }
    }

    fn hex4(&mut self) -> Option<u32> {
        let h = self.text.get(self.at..self.at + 4)?;
        if !h.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        self.at += 4;
        u32::from_str_radix(h, 16).ok()
    }

    /// A string, its escapes read; `None` for a control character, a bad
    /// escape, or a lone surrogate.
    fn string(&mut self) -> Option<String> {
        self.eat(b'"')?;
        let mut out = String::new();
        loop {
            let c = self.text[self.at..].chars().next()?;
            self.at += c.len_utf8();
            match c {
                '"' => return Some(out),
                '\u{0}'..='\u{1f}' => return None,
                '\\' => {
                    let e = self.peek()?;
                    self.at += 1;
                    out.push(match e {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let hi = self.hex4()?;
                            let cp = match hi {
                                0xd800..=0xdbff => {
                                    self.eat(b'\\')?;
                                    self.eat(b'u')?;
                                    let lo = self.hex4()?;
                                    if !(0xdc00..=0xdfff).contains(&lo) {
                                        return None;
                                    }
                                    0x10000 + ((hi - 0xd800) << 10) + (lo - 0xdc00)
                                }
                                0xdc00..=0xdfff => return None,
                                cp => cp,
                            };
                            char::from_u32(cp)?
                        }
                        _ => return None,
                    });
                }
                c => out.push(c),
            }
        }
    }

    /// A number, by RFC 8259's grammar: an integer's text exactly, any other
    /// as its nearest double; `None` when it is not finite.
    fn number(&mut self) -> Option<Datum> {
        let start = self.at;
        self.eat(b'-');
        match self.peek()? {
            b'0' => self.at += 1,
            b'1'..=b'9' => self.digits(),
            _ => return None,
        }
        let mut integer = true;
        if self.eat(b'.').is_some() {
            integer = false;
            if !self.peek()?.is_ascii_digit() {
                return None;
            }
            self.digits();
        }
        if let Some(b'e' | b'E') = self.peek() {
            integer = false;
            self.at += 1;
            if let Some(b'+' | b'-') = self.peek() {
                self.at += 1;
            }
            if !self.peek()?.is_ascii_digit() {
                return None;
            }
            self.digits();
        }
        let t = &self.text[start..self.at];
        if integer && let Ok(i) = t.parse::<i128>() {
            return Some(Datum::Int(i));
        }
        let d = t.parse::<f64>().ok().filter(|d| d.is_finite())?;
        Some(if integer {
            Datum::BigInt(d)
        } else {
            Datum::Float(d)
        })
    }

    fn digits(&mut self) {
        while self.peek().is_some_and(|b| b.is_ascii_digit()) {
            self.at += 1;
        }
    }
}

/// The nearest double to the integer whose big-endian magnitude is `m`,
/// rounding to nearest, ties to even; infinite past the double's range.
fn magnitude_f64(m: &[u8]) -> f64 {
    let m = &m[m.iter().position(|b| *b != 0).unwrap_or(m.len())..];
    let bits = m.len() * 8 - m.first().map_or(8, |b| b.leading_zeros() as usize);
    if bits <= 128 {
        let mut v: u128 = 0;
        for b in m {
            v = (v << 8) | u128::from(*b);
        }
        // `as` rounds to nearest, ties to even.
        return v as f64;
    }
    if bits > 1100 {
        return f64::INFINITY;
    }
    // The top 128 bits, the rest folded into their lowest as a sticky bit:
    // one rounding, then an exact scaling by a power of two.
    let shift = bits - 128;
    let bit = |i: usize| {
        // bit i, counted from the least significant
        let byte = m[m.len() - 1 - i / 8];
        (byte >> (i % 8)) & 1 == 1
    };
    let mut top: u128 = 0;
    for i in (shift..bits).rev() {
        top = (top << 1) | u128::from(bit(i));
    }
    if (0..shift).any(bit) {
        top |= 1;
    }
    let scale = i32::try_from(shift).unwrap_or(i32::MAX);
    (top as f64) * 2f64.powi(scale)
}

/// A bignum (RFC 8949 §3.4.3): tag 2 for `n`, tag 3 for `−1 − n`, `n` the
/// byte string's big-endian magnitude.
pub(crate) fn bignum_of(negative: bool, m: &[u8]) -> Datum {
    let first = m.iter().position(|b| *b != 0).unwrap_or(m.len());
    let m = &m[first..];
    if m.len() <= 16 {
        let mut n: u128 = 0;
        for b in m {
            n = (n << 8) | u128::from(*b);
        }
        let v = if negative {
            i128::try_from(n).ok().map(|n| -1 - n)
        } else {
            i128::try_from(n).ok()
        };
        if let Some(v) = v {
            return Datum::Int(v);
        }
    }
    if negative {
        // −1 − n is −(n + 1).
        let mut up = m.to_vec();
        let mut carry = true;
        for b in up.iter_mut().rev() {
            let (v, c) = b.overflowing_add(u8::from(carry));
            *b = v;
            carry = c;
            if !carry {
                break;
            }
        }
        if carry {
            up.insert(0, 1);
        }
        Datum::BigInt(-magnitude_f64(&up))
    } else {
        Datum::BigInt(magnitude_f64(m))
    }
}

/// One CBOR data item and nothing after it, as a datum, or `None` when the
/// bytes are not one this profile reads (the module doc's table).
#[must_use]
pub fn cbor(bytes: &[u8]) -> Option<Datum> {
    let mut d = ciborium_ll::Decoder::from(bytes);
    let v = cbor_item(&mut d, 0)?;
    (d.offset() == bytes.len()).then_some(v)
}

fn segments_bytes(d: &mut ciborium_ll::Decoder<&[u8]>, len: Option<usize>) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    let mut segs = d.bytes(len);
    while let Some(mut seg) = segs.pull().ok()? {
        while let Some(chunk) = seg.pull(&mut buf).ok()? {
            out.extend_from_slice(chunk);
        }
    }
    Some(out)
}

fn segments_text(d: &mut ciborium_ll::Decoder<&[u8]>, len: Option<usize>) -> Option<String> {
    let mut out = String::new();
    let mut buf = [0u8; 4096];
    let mut segs = d.text(len);
    while let Some(mut seg) = segs.pull().ok()? {
        while let Some(chunk) = seg.pull(&mut buf).ok()? {
            out.push_str(chunk);
        }
    }
    Some(out)
}

fn cbor_item(d: &mut ciborium_ll::Decoder<&[u8]>, depth: usize) -> Option<Datum> {
    use ciborium_ll::{Header, simple};
    if depth > MAX_DEPTH {
        return None;
    }
    Some(match d.pull().ok()? {
        Header::Positive(n) => Datum::Int(i128::from(n)),
        Header::Negative(n) => Datum::Int(-1 - i128::from(n)),
        Header::Float(f) => Datum::Float(f),
        Header::Simple(simple::FALSE) => Datum::Bool(false),
        Header::Simple(simple::TRUE) => Datum::Bool(true),
        Header::Simple(simple::NULL | simple::UNDEFINED) => Datum::Null,
        Header::Simple(_) | Header::Break => return None,
        Header::Bytes(len) => Datum::Bytes(segments_bytes(d, len)?),
        Header::Text(len) => Datum::Text(segments_text(d, len)?),
        Header::Tag(tag) => {
            let next = d.pull().ok()?;
            match (tag, next) {
                (2 | 3, Header::Bytes(len)) => bignum_of(tag == 3, &segments_bytes(d, len)?),
                (_, h) => {
                    d.push(h);
                    cbor_item(d, depth + 1)?
                }
            }
        }
        Header::Array(len) => {
            let mut a = Vec::new();
            match len {
                Some(n) => {
                    for _ in 0..n {
                        a.push(cbor_item(d, depth + 1)?);
                    }
                }
                None => loop {
                    match d.pull().ok()? {
                        Header::Break => break,
                        h => {
                            d.push(h);
                            a.push(cbor_item(d, depth + 1)?);
                        }
                    }
                },
            }
            Datum::Array(a)
        }
        Header::Map(len) => {
            let mut m = BTreeMap::new();
            let mut entry = |d: &mut ciborium_ll::Decoder<&[u8]>| -> Option<()> {
                let Datum::Text(k) = cbor_item(d, depth + 1)? else {
                    return None;
                };
                let v = cbor_item(d, depth + 1)?;
                m.insert(k, v).is_none().then_some(())
            };
            match len {
                Some(n) => {
                    for _ in 0..n {
                        entry(d)?;
                    }
                }
                None => loop {
                    match d.pull().ok()? {
                        Header::Break => break,
                        h => {
                            d.push(h);
                            entry(d)?;
                        }
                    }
                },
            }
            Datum::Map(m)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(h: &str) -> Vec<u8> {
        (0..h.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn json_is_strict() {
        assert_eq!(
            json(br#" {"a": [1, -0, 2.5, true, null, "\u00e9\ud83d\ude00"]} "#),
            {
                let a = Datum::Array(vec![
                    Datum::Int(1),
                    Datum::Int(0),
                    Datum::Float(2.5),
                    Datum::Bool(true),
                    Datum::Null,
                    Datum::Text("é😀".to_owned()),
                ]);
                Some(Datum::Map([("a".to_owned(), a)].into()))
            }
        );
        for bad in [
            &br#"{"a": 1, "a": 2}"#[..],
            br#"{"x": [{"a": 1, "a": 1}]}"#,
            b"1e400",
            br#""\ud800""#,
            br#""\udc00x""#,
            b"1 x",
            b"",
            b"01",
            b"1.",
            b"+1",
            b"[1,]",
            b"\"a\tb\"",
            b"\xff",
            b"NaN",
        ] {
            assert_eq!(json(bad), None, "{}", String::from_utf8_lossy(bad));
        }
        let big = format!("1{}", "0".repeat(400));
        assert_eq!(json(big.as_bytes()), None, "10^400 is past the double");
        assert_eq!(
            json(b"-9223372036854775809"),
            Some(Datum::Int(-9_223_372_036_854_775_809)),
            "exactly"
        );
        let e = format!("1{}", "0".repeat(40));
        assert_eq!(json(e.as_bytes()), Some(Datum::BigInt(1e40)));
        let deep = format!("{}{}", "[".repeat(MAX_DEPTH + 1), "]".repeat(MAX_DEPTH + 1));
        assert_eq!(json(deep.as_bytes()), None);
    }

    #[test]
    fn cbor_reads_bignums_as_integers() {
        // 2^64: tag 2 around 01 00…00 (9 bytes).
        assert_eq!(
            cbor(&hex("c249010000000000000000")),
            Some(Datum::Int(1 << 64))
        );
        assert_eq!(cbor(&hex("c24105")), Some(Datum::Int(5)), "a small one");
        assert_eq!(cbor(&hex("c3420100")), Some(Datum::Int(-257)), "tag 3");
        // Indefinite: 01, then 00.
        assert_eq!(cbor(&hex("c25f41014100ff")), Some(Datum::Int(256)));
        // 17 bytes: 2^128, beyond i128.
        let long = format!("c251{}{}", "01", "00".repeat(16));
        assert_eq!(cbor(&hex(&long)), Some(Datum::BigInt(2f64.powi(128))));
        // 2^127 in 16 bytes is beyond i128 too.
        let edge = format!("c250{}{}", "80", "00".repeat(15));
        assert_eq!(cbor(&hex(&edge)), Some(Datum::BigInt(2f64.powi(127))));
        // tag 1 around an integer: the integer.
        assert_eq!(cbor(&hex("c11a5a5a5a5a")), Some(Datum::Int(0x5a5a_5a5a)));
        // A duplicate key, a key that is not text, trailing bytes.
        assert_eq!(cbor(&hex("a2616101616102")), None);
        assert_eq!(cbor(&hex("a10101")), None);
        assert_eq!(cbor(&hex("0101")), None);
        assert_eq!(
            cbor(&hex("f97e00")).map(|d| matches!(d, Datum::Float(f) if f.is_nan())),
            Some(true)
        );
        assert_eq!(cbor(&hex("f7")), Some(Datum::Null), "undefined is null");
    }

    #[test]
    fn a_long_magnitude_rounds_to_its_nearest_double() {
        // 2^200 + 1: the sticky bit, below every one the double keeps.
        let mut m = vec![0u8; 26];
        m[0] = 1;
        m[25] = 1;
        assert_eq!(magnitude_f64(&m), 2f64.powi(200));
        assert_eq!(magnitude_f64(&[0xff; 200]), f64::INFINITY);
        assert_eq!(magnitude_f64(&[0, 0, 7]), 7.0);
    }
}

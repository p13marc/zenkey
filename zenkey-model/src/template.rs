//! Resource templates: the resource path below a key's kind chunk.
//!
//! - **Literal chunks** follow the plain-chunk charset.
//! - **`{name}`** is one chunk.
//! - **`{name...}`** is a *rest* parameter: one or more chunks, last segment only.
//! - **Values are slugged injectively** when a key is built
//!   ([`crate::slug::chunk_slug`]) and unslugged when one is matched.
//! - **Precedence (r3.3 D1):** when two templates under one kind token
//!   match a key, the most literal wins, chunk by chunk: literal > `{p}` >
//!   `{p...}`.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt;

use thiserror::Error;

use crate::chunk::{is_ident, is_plain_chunk};
use crate::slug::{chunk_slug, chunk_unslug};

/// One segment of a template.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Segment {
    Literal(String),
    Param(String),
    Rest(String),
}

impl Segment {
    fn rank(&self) -> u8 {
        match self {
            Self::Literal(_) => 2,
            Self::Param(_) => 1,
            Self::Rest(_) => 0,
        }
    }
}

/// Why a template string was refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TemplateError {
    #[error("template {0:?} is empty or has an empty segment")]
    Empty(String),
    #[error(
        "template {0:?}: literal segment {1:?} is not a plain chunk, or starts with the slug prefix x-"
    )]
    Literal(String, String),
    #[error("template {0:?}: parameter name {1:?} is not [a-z][a-z0-9_]*")]
    ParamName(String, String),
    #[error("template {0:?}: a rest parameter {{{1}...}} may only be the last segment")]
    RestNotLast(String, String),
    #[error("template {0:?}: parameter {1:?} appears twice")]
    Duplicate(String, String),
}

/// A parsed resource template.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Template {
    raw: String,
    segments: Vec<Segment>,
}

/// Values bound to a template's parameters. A rest parameter binds one or
/// more values.
pub type Bindings = BTreeMap<String, Vec<String>>;

impl Template {
    pub fn parse(raw: &str) -> Result<Self, TemplateError> {
        if raw.is_empty() {
            return Err(TemplateError::Empty(raw.to_owned()));
        }
        let parts: Vec<&str> = raw.split('/').collect();
        let mut segments = Vec::with_capacity(parts.len());
        let mut seen = std::collections::BTreeSet::new();
        for (i, p) in parts.iter().enumerate() {
            if p.is_empty() {
                return Err(TemplateError::Empty(raw.to_owned()));
            }
            let seg = if let Some(inner) = p.strip_prefix('{').and_then(|r| r.strip_suffix('}')) {
                if let Some(name) = inner.strip_suffix("...") {
                    if i != parts.len() - 1 {
                        return Err(TemplateError::RestNotLast(raw.to_owned(), name.to_owned()));
                    }
                    Segment::Rest(name.to_owned())
                } else {
                    Segment::Param(inner.to_owned())
                }
            } else if is_plain_chunk(p) && !p.starts_with("x-") {
                Segment::Literal((*p).to_owned())
            } else {
                return Err(TemplateError::Literal(raw.to_owned(), (*p).to_owned()));
            };
            if let Segment::Param(n) | Segment::Rest(n) = &seg {
                if !is_ident(n) {
                    return Err(TemplateError::ParamName(raw.to_owned(), n.clone()));
                }
                if !seen.insert(n.clone()) {
                    return Err(TemplateError::Duplicate(raw.to_owned(), n.clone()));
                }
            }
            segments.push(seg);
        }
        Ok(Self {
            raw: raw.to_owned(),
            segments,
        })
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    #[must_use]
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    /// Parameter names, in order, with whether each is a rest parameter.
    pub fn params(&self) -> impl Iterator<Item = (&str, bool)> {
        self.segments.iter().filter_map(|s| match s {
            Segment::Param(n) => Some((n.as_str(), false)),
            Segment::Rest(n) => Some((n.as_str(), true)),
            Segment::Literal(_) => None,
        })
    }

    #[must_use]
    pub fn has_params(&self) -> bool {
        self.params().next().is_some()
    }

    /// The template's *shape*: literal chunks kept, parameters erased. Two
    /// templates under one kind token may not share a shape (r3.3 D1).
    #[must_use]
    pub fn shape(&self) -> String {
        self.segments
            .iter()
            .map(|s| match s {
                Segment::Literal(l) => l.as_str(),
                Segment::Param(_) => "{}",
                Segment::Rest(_) => "{...}",
            })
            .collect::<Vec<_>>()
            .join("/")
    }

    /// Builds the resource chunks from raw (unslugged) values. Every value
    /// is slugged; a rest parameter takes one or more values.
    pub fn build(&self, values: &Bindings) -> Result<Vec<String>, String> {
        let mut out = Vec::with_capacity(self.segments.len());
        for s in &self.segments {
            match s {
                Segment::Literal(l) => out.push(l.clone()),
                Segment::Param(n) => match values.get(n).map(Vec::as_slice) {
                    Some([v]) => out.push(chunk_slug(v)),
                    _ => return Err(format!("parameter {n:?} needs exactly one value")),
                },
                Segment::Rest(n) => match values.get(n) {
                    Some(vs) if !vs.is_empty() => out.extend(vs.iter().map(|v| chunk_slug(v))),
                    _ => return Err(format!("rest parameter {n:?} needs one or more values")),
                },
            }
        }
        Ok(out)
    }

    /// Matches resource chunks against this template, returning the
    /// unslugged values. `None` when it does not match, or when a value is
    /// not a canonical slug.
    #[must_use]
    pub fn matches(&self, chunks: &[&str]) -> Option<Bindings> {
        let mut out = Bindings::new();
        let mut i = 0;
        for s in &self.segments {
            match s {
                Segment::Literal(l) => {
                    if chunks.get(i) != Some(&l.as_str()) {
                        return None;
                    }
                    i += 1;
                }
                Segment::Param(n) => {
                    let v = chunk_unslug(chunks.get(i)?)?;
                    out.insert(n.clone(), vec![v]);
                    i += 1;
                }
                Segment::Rest(n) => {
                    let rest = chunks.get(i..)?;
                    if rest.is_empty() {
                        return None;
                    }
                    let vs = rest
                        .iter()
                        .map(|c| chunk_unslug(c))
                        .collect::<Option<Vec<_>>>()?;
                    out.insert(n.clone(), vs);
                    i = chunks.len();
                }
            }
        }
        (i == chunks.len()).then_some(out)
    }

    /// Whether some resource path matches both templates.
    #[must_use]
    pub fn overlaps(&self, other: &Template) -> bool {
        fn go(a: &[Segment], b: &[Segment]) -> bool {
            match (a.first(), b.first()) {
                (None, None) => true,
                // A rest parameter is always the last segment and absorbs one
                // or more chunks, so it overlaps whatever is left on the other
                // side as long as that produces at least one chunk.
                (Some(Segment::Rest(_)), _) => !b.is_empty(),
                (_, Some(Segment::Rest(_))) => !a.is_empty(),
                (None, Some(_)) | (Some(_), None) => false,
                (Some(x), Some(y)) => {
                    let ok = match (x, y) {
                        (Segment::Literal(p), Segment::Literal(q)) => p == q,
                        _ => true,
                    };
                    ok && go(&a[1..], &b[1..])
                }
            }
        }
        go(&self.segments, &other.segments)
    }

    /// Precedence between two templates that both match a key: `Greater`
    /// means `self` wins (it is more literal at the first differing
    /// segment, r3.3 D1).
    #[must_use]
    pub fn precedence(&self, other: &Template) -> Ordering {
        for (a, b) in self.segments.iter().zip(&other.segments) {
            match a.rank().cmp(&b.rank()) {
                Ordering::Equal => {}
                o => return o,
            }
        }
        // Equal prefix: the longer, fully literal-or-param template is the
        // more specific; a template that still has segments beats one that
        // ended in a rest parameter.
        self.segments.len().cmp(&other.segments.len())
    }
}

impl fmt::Display for Template {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

/// Picks the winning template for `chunks` among `candidates`, by D1
/// precedence. Returns its index and bindings.
#[must_use]
pub fn resolve<'a>(
    candidates: impl IntoIterator<Item = &'a Template>,
    chunks: &[&str],
) -> Option<(usize, Bindings)> {
    let mut best: Option<(usize, &Template, Bindings)> = None;
    for (i, t) in candidates.into_iter().enumerate() {
        if let Some(b) = t.matches(chunks) {
            let better = match &best {
                None => true,
                Some((_, bt, _)) => t.precedence(bt) == Ordering::Greater,
            };
            if better {
                best = Some((i, t, b));
            }
        }
    }
    best.map(|(i, _, b)| (i, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Template {
        Template::parse(s).unwrap()
    }

    #[test]
    fn parsing() {
        assert_eq!(
            t("devices/{device}/metrics/{metric...}").shape(),
            "devices/{}/metrics/{...}"
        );
        for bad in ["", "a//b", "A", "{x...}/a", "{X}", "{a}/{a}", "a/{b-c}"] {
            assert!(Template::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn build_and_match_slug() {
        let tpl = t("interfaces/{ns}/{iface}");
        let mut b = Bindings::new();
        b.insert("ns".into(), vec!["default".into()]);
        b.insert("iface".into(), vec!["ETH0".into()]);
        let chunks = tpl.build(&b).unwrap();
        assert_eq!(chunks, ["interfaces", "default", "x-_x45_x54_x480"]);
        let refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
        assert_eq!(tpl.matches(&refs).unwrap(), b);
    }

    #[test]
    fn rest_parameters() {
        let tpl = t("metrics/{metric...}");
        assert!(tpl.matches(&["metrics"]).is_none());
        let m = tpl.matches(&["metrics", "if", "3", "in_octets"]).unwrap();
        assert_eq!(m["metric"], ["if", "3", "in_octets"]);
    }

    #[test]
    fn most_literal_first() {
        let catch = t("flow/{x}");
        let lit = t("flow/duration_p50_ms");
        let rest = t("flow/{x...}");
        let cands = [catch.clone(), lit.clone(), rest.clone()];
        assert_eq!(resolve(&cands, &["flow", "duration_p50_ms"]).unwrap().0, 1);
        assert_eq!(resolve(&cands, &["flow", "other"]).unwrap().0, 0);
        assert_eq!(resolve(&cands, &["flow", "a", "b"]).unwrap().0, 2);
        assert!(catch.overlaps(&lit) && lit.overlaps(&rest) && catch.overlaps(&rest));
        assert!(!t("a/{x}").overlaps(&t("b/{x}")));
        assert!(!t("a/{x}").overlaps(&t("a/{x}/c")));
        assert!(t("a/{x...}").overlaps(&t("a/b/c")));
    }

    #[test]
    fn shapes_collide_regardless_of_param_names() {
        assert_eq!(t("action/{name}").shape(), t("action/{event_id}").shape());
    }
}

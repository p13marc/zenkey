//! Checking a value against a JSON Schema type of a bundle (spec `core.md`
//! §7.3), for tools that build a payload from JSON and were never compiled
//! against the contract (#671).
//!
//! The subset is small on purpose (§7.3): `type`, `properties`,
//! `required`, `additionalProperties`, `items`, `prefixItems`, `enum`,
//! `const`, the numeric bounds, `minLength`/`maxLength`,
//! `minItems`/`maxItems`, `$ref` and `oneOf`/`anyOf`, plus boolean
//! schemas. None of them needs a regular-expression dialect, which is why
//! the subset refuses `pattern`. This module implements those keywords as
//! JSON Schema 2020-12 defines them, and nothing else: an annotation is
//! ignored, and a keyword outside the subset cannot occur, because a
//! contract that uses one does not load (E037).
//!
//! The core requires no owner to evaluate a schema (§5.1, "The request"),
//! and a caller MUST NOT depend on a refusal of what the schema refuses: a
//! writer sends only what its schema declares (§9.8). This is how a tool
//! keeps that side of the rule before it sends.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::bundle::Bundle;

/// One way a value fails its schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// Where in the value, as a JSON pointer (`""` is the value itself).
    pub at: String,
    /// What the schema asked for there.
    pub why: String,
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let at = if self.at.is_empty() { "/" } else { &self.at };
        write!(f, "{at}: {}", self.why)
    }
}

/// Checks `value` against the JSON Schema type `ty`, a canonical type
/// reference of `bundle` (`{"kind": "jsonschema", "name", "schema"}`).
/// `Err` lists every violation found, in the order met; a type that is not
/// a JSON Schema type of the bundle, or a `$ref` that resolves to nothing,
/// is one violation at the root.
pub fn validate(bundle: &Bundle, ty: &Value, value: &Value) -> Result<(), Vec<Violation>> {
    let at_root = |why: String| {
        Err(vec![Violation {
            at: String::new(),
            why,
        }])
    };
    if ty["kind"] != "jsonschema" {
        return at_root(format!("{ty} is not a JSON Schema type"));
    }
    let docs = Docs::of(bundle);
    let (Some(id), Some(name)) = (ty["schema"].as_str(), ty["name"].as_str()) else {
        return at_root(format!("{ty} is not a type reference"));
    };
    let Some(doc) = bundle.schemas.get(id).map(|s| &s["data"]) else {
        return at_root(format!("the bundle holds no schema {id}"));
    };
    let Some(schema) = doc.pointer(&format!("/$defs/{}", escape(name))) else {
        return at_root(format!("{name} is not defined in its schema"));
    };
    let mut out = Vec::new();
    Cx { docs: &docs }.check(schema, doc, value, "", &mut out, 0);
    if out.is_empty() { Ok(()) } else { Err(out) }
}

/// The bundle's JSON Schema documents by artifact name, the stem a `$ref`'s
/// file part names (§9.4).
struct Docs<'b>(BTreeMap<&'b str, &'b Value>);

impl<'b> Docs<'b> {
    fn of(bundle: &'b Bundle) -> Self {
        let names: BTreeMap<&str, &str> = bundle.contract["schemas"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| Some((s["id"].as_str()?, s["name"].as_str()?)))
            .collect();
        Self(
            bundle
                .schemas
                .iter()
                .filter(|(_, s)| s["kind"] == "jsonschema")
                .filter_map(|(id, s)| Some((*names.get(id.as_str())?, &s["data"])))
                .collect(),
        )
    }
}

struct Cx<'d, 'b> {
    docs: &'d Docs<'b>,
}

/// How deep `$ref`s may nest while checking one value: a recursive type
/// recurses with the value, so this bounds only a `$ref` cycle that
/// consumes nothing (`{"$ref": "#"}`).
const MAX_DEPTH: usize = 256;

impl<'b> Cx<'_, 'b> {
    #[allow(clippy::too_many_lines)]
    fn check(
        &self,
        schema: &Value,
        doc: &'b Value,
        v: &Value,
        at: &str,
        out: &mut Vec<Violation>,
        depth: usize,
    ) {
        let mut fail = |why: String| {
            out.push(Violation {
                at: at.to_owned(),
                why,
            });
        };
        match schema {
            Value::Bool(true) => return,
            Value::Bool(false) => return fail("no value is allowed here".to_owned()),
            Value::Object(_) => {}
            _ => return fail("the schema is not a schema".to_owned()),
        }
        if depth > MAX_DEPTH {
            return fail("`$ref`s nest too deep".to_owned());
        }
        // `$ref` applies beside its siblings (2020-12).
        if let Some(r) = schema.get("$ref").and_then(Value::as_str) {
            match self.resolve(r, doc) {
                Some((target, tdoc)) => self.check(target, tdoc, v, at, out, depth + 1),
                None => {
                    return out.push(Violation {
                        at: at.to_owned(),
                        why: format!("`$ref` {r:?} resolves to nothing"),
                    });
                }
            }
        }
        let mut fail = |why: String| {
            out.push(Violation {
                at: at.to_owned(),
                why,
            });
        };
        if let Some(t) = schema.get("type") {
            let names: Vec<&str> = match t {
                Value::String(s) => vec![s.as_str()],
                Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
                _ => Vec::new(),
            };
            if !names.iter().any(|n| is_type(v, n)) {
                fail(format!(
                    "expected {}, found {}",
                    names.join(" or "),
                    type_name(v)
                ));
            }
        }
        if let Some(e) = schema.get("enum").and_then(Value::as_array)
            && !e.iter().any(|x| same(x, v))
        {
            fail(format!("{v} is not one of {}", Value::Array(e.clone())));
        }
        if let Some(c) = schema.get("const")
            && !same(c, v)
        {
            fail(format!("expected {c}, found {v}"));
        }
        if let Some(n) = v.as_f64().filter(|_| v.is_number()) {
            let bound = |k: &str| schema.get(k).and_then(Value::as_f64);
            if let Some(m) = bound("minimum")
                && n < m
            {
                fail(format!("{v} is below the minimum {m}"));
            }
            if let Some(m) = bound("maximum")
                && n > m
            {
                fail(format!("{v} is above the maximum {m}"));
            }
            if let Some(m) = bound("exclusiveMinimum")
                && n <= m
            {
                fail(format!("{v} is not above {m}"));
            }
            if let Some(m) = bound("exclusiveMaximum")
                && n >= m
            {
                fail(format!("{v} is not below {m}"));
            }
        }
        let count = |k: &str| schema.get(k).and_then(Value::as_u64);
        if let Value::String(s) = v {
            // Lengths count code points, as JSON Schema does.
            let n = s.chars().count() as u64;
            if let Some(m) = count("minLength")
                && n < m
            {
                fail(format!("{n} characters, fewer than {m}"));
            }
            if let Some(m) = count("maxLength")
                && n > m
            {
                fail(format!("{n} characters, more than {m}"));
            }
        }
        if let Value::Array(items) = v {
            let n = items.len() as u64;
            if let Some(m) = count("minItems")
                && n < m
            {
                fail(format!("{n} items, fewer than {m}"));
            }
            if let Some(m) = count("maxItems")
                && n > m
            {
                fail(format!("{n} items, more than {m}"));
            }
            let prefix = schema
                .get("prefixItems")
                .and_then(Value::as_array)
                .map_or(&[][..], Vec::as_slice);
            for (i, x) in items.iter().enumerate() {
                let sub = prefix.get(i).or_else(|| schema.get("items"));
                if let Some(sub) = sub {
                    self.check(sub, doc, x, &format!("{at}/{i}"), out, depth);
                }
            }
        }
        if let Value::Object(m) = v {
            let props = schema.get("properties").and_then(Value::as_object);
            for r in schema
                .get("required")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                if !m.contains_key(r) {
                    out.push(Violation {
                        at: at.to_owned(),
                        why: format!("the required property {r:?} is missing"),
                    });
                }
            }
            for (k, x) in m {
                let at = format!("{at}/{}", escape(k));
                match props.and_then(|p| p.get(k)) {
                    Some(sub) => self.check(sub, doc, x, &at, out, depth),
                    None => {
                        if let Some(extra) = schema.get("additionalProperties") {
                            self.check(extra, doc, x, &at, out, depth);
                        }
                    }
                }
            }
        }
        for (k, exactly_one) in [("oneOf", true), ("anyOf", false)] {
            let Some(branches) = schema.get(k).and_then(Value::as_array) else {
                continue;
            };
            let passing = branches
                .iter()
                .filter(|b| {
                    let mut scratch = Vec::new();
                    self.check(b, doc, v, at, &mut scratch, depth + 1);
                    scratch.is_empty()
                })
                .count();
            let ok = if exactly_one {
                passing == 1
            } else {
                passing >= 1
            };
            if !ok {
                out.push(Violation {
                    at: at.to_owned(),
                    why: format!(
                        "`{k}`: {passing} of {} branches match, {}",
                        branches.len(),
                        if exactly_one {
                            "one must"
                        } else {
                            "at least one must"
                        }
                    ),
                });
            }
        }
    }

    /// Resolves a `$ref` from `doc`: the file part names an artifact by its
    /// stem (§9.4: in a bundle, the last path segment, without `.json`),
    /// and an empty one names `doc` itself.
    fn resolve(&self, r: &str, doc: &'b Value) -> Option<(&'b Value, &'b Value)> {
        let (file, ptr) = r.split_once('#').unwrap_or((r, ""));
        let doc = if file.is_empty() {
            doc
        } else {
            let base = file.rsplit('/').next().unwrap_or(file);
            let stem = base.strip_suffix(".json").unwrap_or(base);
            self.docs.0.get(stem).copied()?
        };
        Some((doc.pointer(ptr)?, doc))
    }
}

/// Whether `v` is of the JSON Schema type `name`. An integer is any number
/// with no fractional part, `1.0` included (2020-12).
fn is_type(v: &Value, name: &str) -> bool {
    match name {
        "null" => v.is_null(),
        "boolean" => v.is_boolean(),
        "object" => v.is_object(),
        "array" => v.is_array(),
        "string" => v.is_string(),
        "number" => v.is_number(),
        "integer" => {
            v.is_i64()
                || v.is_u64()
                || v.as_f64()
                    .is_some_and(|f| f.is_finite() && f.fract() == 0.0)
        }
        _ => false,
    }
}

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_f64() => "number",
        Value::Number(_) => "integer",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// JSON Schema equality: numbers by value (`1` is `1.0`), everything else
/// structurally.
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => match (x.as_i64(), y.as_i64()) {
            (Some(i), Some(j)) => i == j,
            _ => match (x.as_u64(), y.as_u64()) {
                (Some(i), Some(j)) => i == j,
                _ => x.as_f64() == y.as_f64(),
            },
        },
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(a, b)| same(a, b))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, a)| y.get(k).is_some_and(|b| same(a, b)))
        }
        _ => a == b,
    }
}

/// A JSON pointer token (RFC 6901).
fn escape(s: &str) -> String {
    s.replace('~', "~0").replace('/', "~1")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::validate;
    use crate::bundle::Bundle;

    fn bundle(toml: &str, files: &[(&str, &str)]) -> Bundle {
        let dir = std::env::temp_dir().join(format!(
            "zk2-validate-{}-{}",
            std::process::id(),
            toml.len()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        for (name, text) in files {
            std::fs::write(dir.join(name), text).unwrap();
        }
        let l = crate::contract::load_str(toml, &dir, None);
        Bundle::build(&l.contract.unwrap_or_else(|| panic!("{}", l.report)))
    }

    fn status() -> Bundle {
        bundle(
            "[interface]\nname = \"m\"\nmajor = 1\nminor = 0\n\
             [schemas]\njsonschema = [\"s.json\", \"common.json\"]\n\
             [resources.status]\nkind = \"state\"\ntype = \"json:Status\"\n",
            &[
                (
                    "s.json",
                    r#"{"$defs": {"Status": {
                        "type": "object",
                        "properties": {
                            "state": {"type": "string", "enum": ["up", "down"]},
                            "since_ms": {"type": "integer", "minimum": 0},
                            "label": {"type": "string", "maxLength": 3},
                            "link": {"$ref": "common.json#/$defs/Link"},
                            "hops": {"type": "array", "prefixItems": [{"type": "string"}],
                                     "items": {"type": "integer"}, "maxItems": 3},
                            "retries": {"anyOf": [{"type": "integer"}, {"type": "null"}]},
                            "event": {"oneOf": [
                                {"type": "object", "properties": {"up": {"const": true}}, "required": ["up"]},
                                {"type": "object", "properties": {"down": {"const": true}}, "required": ["down"]}
                            ]}
                        },
                        "required": ["state"],
                        "additionalProperties": false
                    }}}"#,
                ),
                (
                    "common.json",
                    r#"{"$defs": {"Link": {"type": "object",
                        "properties": {"mbps": {"type": "number", "exclusiveMinimum": 0}},
                        "required": ["mbps"]}}}"#,
                ),
            ],
        )
    }

    fn check(b: &Bundle, v: serde_json::Value) -> Result<(), Vec<String>> {
        let ty = crate::decode::type_of(b, "state", "status", "type").unwrap();
        validate(b, ty, &v).map_err(|e| e.iter().map(ToString::to_string).collect())
    }

    #[test]
    fn a_value_its_schema_declares_passes() {
        let b = status();
        let ok = json!({
            "state": "up", "since_ms": 5.0, "label": "été", "link": {"mbps": 0.5},
            "hops": ["a", 1, 2], "retries": null, "event": {"up": true}
        });
        assert_eq!(check(&b, ok), Ok(()));
        assert_eq!(check(&b, json!({"state": "down"})), Ok(()));
    }

    #[test]
    fn each_keyword_of_the_subset_refuses_what_it_refuses() {
        let b = status();
        let cases = [
            (json!({}), "/: the required property \"state\" is missing"),
            (
                json!({"state": "sideways"}),
                "/state: \"sideways\" is not one of",
            ),
            (
                json!({"state": 1}),
                "/state: expected string, found integer",
            ),
            (
                json!({"state": "up", "since_ms": -1}),
                "/since_ms: -1 is below the minimum 0",
            ),
            (
                json!({"state": "up", "since_ms": 1.5}),
                "/since_ms: expected integer, found number",
            ),
            (
                json!({"state": "up", "label": "abcd"}),
                "/label: 4 characters, more than 3",
            ),
            (
                json!({"state": "up", "link": {}}),
                "/link: the required property \"mbps\" is missing",
            ),
            (
                json!({"state": "up", "link": {"mbps": 0}}),
                "/link/mbps: 0 is not above 0",
            ),
            (
                json!({"state": "up", "hops": [1]}),
                "/hops/0: expected string, found integer",
            ),
            (
                json!({"state": "up", "hops": ["a", "b"]}),
                "/hops/1: expected integer, found string",
            ),
            (
                json!({"state": "up", "hops": ["a", 1, 2, 3]}),
                "/hops: 4 items, more than 3",
            ),
            (
                json!({"state": "up", "retries": "x"}),
                "/retries: `anyOf`: 0 of 2 branches match",
            ),
            (
                json!({"state": "up", "event": {}}),
                "/event: `oneOf`: 0 of 2 branches match",
            ),
            (
                json!({"state": "up", "event": {"up": true, "down": true}}),
                "/event: `oneOf`: 2 of 2",
            ),
            (
                json!({"state": "up", "extra": 1}),
                "/extra: no value is allowed here",
            ),
        ];
        for (v, want) in cases {
            let got = check(&b, v.clone()).expect_err(&v.to_string());
            assert!(
                got.iter().any(|g| g.starts_with(want)),
                "{v}: {got:?}, want {want:?}"
            );
        }
    }

    #[test]
    fn a_type_that_is_not_json_schema_is_one_violation() {
        let b = status();
        let raw = json!({"kind": "raw", "media_type": "text/plain"});
        let err = validate(&b, &raw, &json!("x")).unwrap_err();
        assert_eq!(err.len(), 1);
        assert_eq!(err[0].at, "");
    }
}

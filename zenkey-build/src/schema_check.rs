//! The Rust-first path's guard (G23, Z19): a type named by a hint and the
//! schema committed for it must say the same thing, read through the zk2
//! JSON Schema subset (spec §7.3).

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::{Map, Value};
use zenkey_model::compat::nullable_reading;

/// The subset's keywords (spec §7.3), as `zenkey-model` lints them (E037).
const SUBSET: &[&str] = &[
    "type",
    "properties",
    "required",
    "additionalProperties",
    "items",
    "prefixItems",
    "enum",
    "const",
    "minimum",
    "maximum",
    "exclusiveMinimum",
    "exclusiveMaximum",
    "minLength",
    "maxLength",
    "minItems",
    "maxItems",
    "$ref",
    "oneOf",
    "anyOf",
];

/// Annotations: carried and ignored, so never compared.
const ANNOTATIONS: &[&str] = &[
    "$schema",
    "$id",
    "$defs",
    "$comment",
    "title",
    "description",
    "default",
    "examples",
    "format",
    "deprecated",
    "readOnly",
    "writeOnly",
];

/// What the subset says of a schema: its subset keywords only, recursively,
/// with `required` and a `type` list compared as sets. Keywords outside
/// both the subset and the annotations are collected in `outside`.
fn project(v: &Value, outside: &mut BTreeSet<String>) -> Value {
    let Value::Object(m) = v else {
        return v.clone();
    };
    let mut out = Map::new();
    for (k, x) in m {
        if !SUBSET.contains(&k.as_str()) {
            if !ANNOTATIONS.contains(&k.as_str()) {
                outside.insert(k.clone());
            }
            continue;
        }
        let x = match (k.as_str(), x) {
            ("properties", Value::Object(props)) => Value::Object(
                props
                    .iter()
                    .map(|(p, s)| (p.clone(), project(s, outside)))
                    .collect(),
            ),
            ("prefixItems" | "oneOf" | "anyOf", Value::Array(subs)) => {
                Value::Array(subs.iter().map(|s| project(s, outside)).collect())
            }
            ("items" | "additionalProperties", sub) => project(sub, outside),
            ("required" | "type", Value::Array(a)) => {
                let mut a = a.clone();
                a.sort_by_key(ToString::to_string);
                Value::Array(a)
            }
            (_, other) => other.clone(),
        };
        out.insert(k.clone(), x);
    }
    Value::Object(out)
}

fn json_type(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// The target of a nullable form's S branch, for [`nullable_reading`]: the
/// branch itself, or the definition its local `$ref` names, with that
/// name. `None` (the form compared as written) for a `$ref` with keywords
/// beside it, one that names no definition, or one already being followed.
fn local_target(
    branch: &Value,
    defs: &Map<String, Value>,
    stack: &[String],
) -> Option<(Value, Option<String>)> {
    let Some(r) = branch.get("$ref") else {
        return Some((branch.clone(), None));
    };
    let name = r.as_str()?.strip_prefix("#/$defs/")?;
    if branch.as_object()?.len() != 1 || stack.iter().any(|s| s == name) {
        return None;
    }
    Some((defs.get(name)?.clone(), Some(name.to_owned())))
}

/// What a projected schema means, so two spellings of one meaning compare
/// equal: a local `$ref` is replaced by the definition it names (a cycle
/// keeps its `$ref`), a `type` that an `enum` or `const` already implies
/// is dropped (`{"type": "string", "enum": ["a"]}` is `{"enum": ["a"]}`),
/// and a nullable form is read as its reading (spec §7.3, 0.7, C-1):
/// `{"anyOf": [S, {"type": "null"}]}` is S with `null` added to its `type`
/// and its `enum`, by `zenkey-model`'s reading, the classifier's own
/// ([`nullable_reading`]). So schemars's `{"type": ["integer", "null"]}`
/// and another generator's `{"anyOf": [{"type": "integer"}, {"type":
/// "null"}]}` are one type.
fn normalize(v: &Value, defs: &Map<String, Value>, stack: &mut Vec<String>) -> Value {
    if let Some((read, name)) = nullable_reading(v, |b| local_target(b, defs, stack)) {
        stack.extend(name.clone());
        let out = normalize(&read, defs, stack);
        if name.is_some() {
            stack.pop();
        }
        return out;
    }
    match v {
        Value::Object(m) => {
            if let Some(Value::String(r)) = m.get("$ref")
                && m.len() == 1
                && let Some(name) = r.strip_prefix("#/$defs/")
                && let Some(target) = defs.get(name)
                && !stack.iter().any(|s| s == name)
            {
                stack.push(name.to_owned());
                let out = normalize(target, defs, stack);
                stack.pop();
                return out;
            }
            let mut out: Map<String, Value> = m
                .iter()
                .map(|(k, x)| {
                    let x = match k.as_str() {
                        "enum" | "const" | "required" | "type" => x.clone(),
                        _ => normalize(x, defs, stack),
                    };
                    (k.clone(), x)
                })
                .collect();
            let implied = |t: &str| {
                let values: Vec<&Value> = match (m.get("enum"), m.get("const")) {
                    (Some(Value::Array(a)), _) => a.iter().collect(),
                    (None, Some(c)) => vec![c],
                    _ => return false,
                };
                values
                    .iter()
                    .all(|v| json_type(v) == t || (t == "number" && json_type(v) == "integer"))
            };
            if let Some(Value::String(t)) = out.get("type")
                && implied(t)
            {
                out.remove("type");
            }
            Value::Object(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(|x| normalize(x, defs, stack)).collect()),
        other => other.clone(),
    }
}

/// Checks that `T`'s schemars schema and the committed schema `file`'s
/// `$defs/<name>` agree, read through the zk2 subset (spec §7.3): what each
/// says once its local `$ref`s are followed, so a definition inlined on one
/// side and referenced on the other is the same, and once a nullable form is
/// read as its reading (§7.3, 0.7), so `Option<T>` spelled `{"type": [T,
/// "null"]}` on one side and `{"anyOf": [S, {"type": "null"}]}` on the
/// other is the same. Annotations (titles,
/// descriptions, `format`) are ignored, and so is a `type` an `enum` already
/// implies; a keyword schemars emits outside the subset is a failure, as the
/// lints would refuse it (E037).
///
/// A test helper for the Rust-first path: a type bound by
/// [`crate::Config::json_type`] and the schema committed from it.
///
/// ```ignore
/// #[test]
/// fn status_matches_its_schema() {
///     zenkey_build::check_schema::<crate::model::Status>("contracts/schemas/status.json", "Status")
///         .unwrap();
/// }
/// ```
///
/// # Errors
/// Every disagreement, one per line.
pub fn check_schema<T: schemars::JsonSchema>(
    file: impl AsRef<Path>,
    name: &str,
) -> Result<(), String> {
    let file = file.as_ref();
    let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let committed: Value =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))?;
    let cdefs = committed
        .get("$defs")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{}: no $defs", file.display()))?;
    let generated = serde_json::to_value(schemars::schema_for!(T))
        .map_err(|e| format!("the schemars schema does not serialize: {e}"))?;
    let gdefs = generated
        .get("$defs")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    let want = cdefs
        .get(name)
        .ok_or_else(|| format!("{}: no $defs/{name}", file.display()))?;
    let mut outside = BTreeSet::new();
    let mut ignored = BTreeSet::new();
    let gdefs: Map<String, Value> = gdefs
        .iter()
        .map(|(k, v)| (k.clone(), project(v, &mut outside)))
        .collect();
    let cdefs: Map<String, Value> = cdefs
        .iter()
        .map(|(k, v)| (k.clone(), project(v, &mut ignored)))
        .collect();
    let g = normalize(
        &project(&generated, &mut outside),
        &gdefs,
        &mut vec![name.to_owned()],
    );
    let c = normalize(
        &project(want, &mut ignored),
        &cdefs,
        &mut vec![name.to_owned()],
    );
    let mut problems = Vec::new();
    if g != c {
        problems.push(format!("the type says {g}\nthe committed schema says {c}"));
    }
    for k in outside {
        problems.push(format!(
            "schemars emits {k:?}, outside the zk2 JSON Schema subset (spec §7.3, E037)"
        ));
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} vs {name}:\n{}",
            file.display(),
            problems.join("\n")
        ))
    }
}

#[cfg(test)]
mod tests {
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};

    use super::check_schema;

    #[derive(Serialize, Deserialize, JsonSchema)]
    #[allow(dead_code)]
    struct Reading {
        /// A comment is an annotation.
        value: f64,
        unit: Option<String>,
        source: Source,
    }

    #[derive(Serialize, Deserialize, JsonSchema)]
    #[allow(dead_code)]
    enum Source {
        Probe,
        Model,
    }

    fn committed(
        dir: &std::path::Path,
        edit: impl FnOnce(&mut serde_json::Value),
    ) -> std::path::PathBuf {
        let mut root = serde_json::to_value(schemars::schema_for!(Reading)).unwrap();
        let mut defs = root["$defs"].take();
        root.as_object_mut().unwrap().remove("$defs");
        root.as_object_mut().unwrap().remove("$schema");
        defs["Reading"] = root;
        let mut doc = serde_json::json!({"$defs": defs});
        edit(&mut doc);
        let p = dir.join("reading.json");
        std::fs::write(&p, serde_json::to_vec_pretty(&doc).unwrap()).unwrap();
        p
    }

    #[test]
    fn a_type_and_its_committed_schema_agree_through_the_subset() {
        let dir = std::env::temp_dir().join(format!("zk2-check-schema-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ok = committed(&dir, |d| {
            d["$defs"]["Reading"]["description"] = "annotations do not count".into();
        });
        check_schema::<Reading>(&ok, "Reading").unwrap();
        let drift = committed(&dir, |d| {
            d["$defs"]["Reading"]["properties"]
                .as_object_mut()
                .unwrap()
                .remove("unit");
        });
        let e = check_schema::<Reading>(&drift, "Reading").unwrap_err();
        assert!(e.contains("the type says"), "{e}");
        let renamed = committed(&dir, |d| {
            d["$defs"]["Source"]["enum"] = serde_json::json!(["Probe", "Simulation"]);
        });
        let e = check_schema::<Reading>(&renamed, "Reading").unwrap_err();
        assert!(e.contains("Simulation"), "{e}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[derive(Serialize, Deserialize, JsonSchema)]
    #[allow(dead_code)]
    struct Probe {
        count: Option<u32>,
        source: Option<Source>,
    }

    /// `Probe`'s schemars schema, committed after `edit`.
    fn committed_probe(
        dir: &std::path::Path,
        edit: impl FnOnce(&mut serde_json::Map<String, serde_json::Value>),
    ) -> std::path::PathBuf {
        let mut root = serde_json::to_value(schemars::schema_for!(Probe)).unwrap();
        let mut defs = root["$defs"].take();
        root.as_object_mut().unwrap().remove("$defs");
        root.as_object_mut().unwrap().remove("$schema");
        edit(root["properties"].as_object_mut().unwrap());
        defs["Probe"] = root;
        let p = dir.join("probe.json");
        let doc = serde_json::json!({"$defs": defs});
        std::fs::write(&p, serde_json::to_vec_pretty(&doc).unwrap()).unwrap();
        p
    }

    /// Spec §7.3 (0.7, C-1, #660): a nullable's two spellings are one type
    /// to the check, read as the classifier reads them.
    #[test]
    fn a_nullable_is_one_type_in_either_spelling() {
        use serde_json::json;
        let generated = serde_json::to_value(schemars::schema_for!(Probe)).unwrap();
        // What schemars 1 writes, the two spellings the rule names.
        assert_eq!(
            generated["properties"]["count"]["type"],
            json!(["integer", "null"])
        );
        assert_eq!(
            generated["properties"]["source"],
            json!({"anyOf": [{"$ref": "#/$defs/Source"}, {"type": "null"}]})
        );
        let dir = std::env::temp_dir().join(format!("zk2-check-nullable-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // The other spelling of each, committed: the same type.
        let swapped = committed_probe(&dir, |p| {
            p["count"] = json!({"anyOf": [{"type": "integer", "minimum": 0}, {"type": "null"}]});
            p["source"] = json!({"type": ["string", "null"], "enum": ["Probe", "Model", null]});
        });
        check_schema::<Probe>(&swapped, "Probe").unwrap();
        // A change inside S is still a change.
        let retyped = committed_probe(&dir, |p| {
            p["count"] = json!({"anyOf": [{"type": "string"}, {"type": "null"}]});
        });
        let e = check_schema::<Probe>(&retyped, "Probe").unwrap_err();
        assert!(e.contains("the type says"), "{e}");
        // So is a null dropped: the type no longer admits one.
        let required = committed_probe(&dir, |p| {
            p["source"] = json!({"type": "string", "enum": ["Probe", "Model"]});
        });
        assert!(check_schema::<Probe>(&required, "Probe").is_err());
        // An `anyOf` that is not the form is compared as written.
        let other = committed_probe(&dir, |p| {
            p["count"] = json!({"anyOf": [{"type": "integer", "minimum": 0}, {"type": "string"}]});
        });
        assert!(check_schema::<Probe>(&other, "Probe").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

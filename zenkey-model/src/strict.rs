//! Strict JSON: duplicate object keys are an error, not "last one wins"
//! (r3.1, the canonical-form restrictions). Bundles and JSON Schema
//! sources are read through here.

use std::fmt;

use serde::de::{self, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Value};

/// Parses JSON, refusing duplicate keys at any depth.
pub fn parse_json(text: &str) -> Result<Value, serde_json::Error> {
    let mut de = serde_json::Deserializer::from_str(text);
    let v = StrictValue.deserialize(&mut de)?;
    de.end()?;
    Ok(v)
}

struct StrictValue;

impl<'de> DeserializeSeed<'de> for StrictValue {
    type Value = Value;
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
        d.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for StrictValue {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON value")
    }
    fn visit_bool<E>(self, v: bool) -> Result<Value, E> {
        Ok(Value::Bool(v))
    }
    fn visit_i64<E>(self, v: i64) -> Result<Value, E> {
        Ok(v.into())
    }
    fn visit_u64<E>(self, v: u64) -> Result<Value, E> {
        Ok(v.into())
    }
    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Value, E> {
        serde_json::Number::from_f64(v)
            .map(Value::Number)
            .ok_or_else(|| E::custom("a number that is not finite"))
    }
    fn visit_str<E>(self, v: &str) -> Result<Value, E> {
        Ok(Value::String(v.to_owned()))
    }
    fn visit_string<E>(self, v: String) -> Result<Value, E> {
        Ok(Value::String(v))
    }
    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_none<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut v = Vec::new();
        while let Some(x) = seq.next_element_seed(StrictValue)? {
            v.push(x);
        }
        Ok(Value::Array(v))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut m = Map::new();
        while let Some(k) = map.next_key::<String>()? {
            if m.contains_key(&k) {
                return Err(de::Error::custom(format!("duplicate key {k:?}")));
            }
            let v = map.next_value_seed(StrictValue)?;
            m.insert(k, v);
        }
        Ok(Value::Object(m))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicates_are_refused_at_any_depth() {
        assert!(parse_json(r#"{"a":1,"b":{"c":2}}"#).is_ok());
        assert!(parse_json(r#"{"a":1,"a":2}"#).is_err());
        assert!(parse_json(r#"{"x":[{"a":1,"a":1}]}"#).is_err());
        assert!(parse_json(r#"{"a":1} x"#).is_err());
    }
}

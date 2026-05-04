//! `SpoeValue` → OpenTelemetry attribute conversion (task T006).
//!
//! Implements the typing table from `specs/001-otel-plugin/research.md`
//! item R10. Operator-specified output types coerce values; mismatches
//! increment the per-rule error counter and the attribute is omitted.

use haproxy_spoa_hub_plugin_api::SpoeValue;
use opentelemetry::{Key, KeyValue, Value};
use std::net::{Ipv4Addr, Ipv6Addr};

/// Operator-specified target type for an attribute. Optional; when absent
/// the natural mapping is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AttributeType {
    String,
    Int,
    Double,
    Bool,
}

/// Convert a `SpoeValue` into an OpenTelemetry `Value` using the natural
/// mapping (no coercion). Returns `None` for `Null` (the attribute is
/// omitted upstream).
#[must_use]
pub fn natural(value: &SpoeValue) -> Option<Value> {
    match value {
        SpoeValue::Null => None,
        SpoeValue::Bool(b) => Some(Value::Bool(*b)),
        SpoeValue::Int32(i) => Some(Value::I64(i64::from(*i))),
        SpoeValue::Uint32(u) => Some(Value::I64(i64::from(*u))),
        SpoeValue::Int64(i) => Some(Value::I64(*i)),
        SpoeValue::Uint64(u) => i64::try_from(*u).ok().map(Value::I64),
        SpoeValue::Ipv4(o) => Some(Value::String(Ipv4Addr::from(*o).to_string().into())),
        SpoeValue::Ipv6(o) => Some(Value::String(Ipv6Addr::from(*o).to_string().into())),
        SpoeValue::String(s) => Some(Value::String(s.as_str().to_owned().into())),
        SpoeValue::Binary(b) => Some(Value::String(hex::encode(b.as_slice()).into())),
        SpoeValue::__Other(_, _) => None,
    }
}

/// Convert with operator-specified coercion. Returns `None` if the value
/// cannot be coerced to the requested type (caller increments an error
/// counter and omits the attribute).
#[must_use]
pub fn coerce(value: &SpoeValue, target: AttributeType) -> Option<Value> {
    let natural = natural(value)?;
    match (target, &natural) {
        (AttributeType::String, _) => Some(Value::String(stringify(&natural).into())),
        (AttributeType::Int, Value::I64(_)) => Some(natural),
        (AttributeType::Int, Value::Bool(b)) => Some(Value::I64(i64::from(*b))),
        (AttributeType::Int, Value::F64(f)) => Some(Value::I64(*f as i64)),
        (AttributeType::Int, Value::String(s)) => s.as_str().parse::<i64>().ok().map(Value::I64),
        (AttributeType::Double, Value::F64(_)) => Some(natural),
        (AttributeType::Double, Value::I64(i)) => Some(Value::F64(*i as f64)),
        (AttributeType::Double, Value::Bool(b)) => Some(Value::F64(if *b { 1.0 } else { 0.0 })),
        (AttributeType::Double, Value::String(s)) => s.as_str().parse::<f64>().ok().map(Value::F64),
        (AttributeType::Bool, Value::Bool(_)) => Some(natural),
        (AttributeType::Bool, Value::I64(i)) => Some(Value::Bool(*i != 0)),
        (AttributeType::Bool, Value::F64(f)) => Some(Value::Bool(*f != 0.0)),
        (AttributeType::Bool, Value::String(s)) => match s.as_str() {
            "true" => Some(Value::Bool(true)),
            "false" => Some(Value::Bool(false)),
            _ => None,
        },
        _ => None,
    }
}

/// Build a `KeyValue` from a key and a `SpoeValue`, applying the optional
/// coercion target. Returns `None` when the value should be omitted.
#[must_use]
pub fn key_value(
    key: impl Into<Key>,
    value: &SpoeValue,
    target: Option<AttributeType>,
) -> Option<KeyValue> {
    let v = match target {
        Some(t) => coerce(value, t)?,
        None => natural(value)?,
    };
    Some(KeyValue::new(key, v))
}

fn stringify(v: &Value) -> String {
    match v {
        Value::Bool(b) => b.to_string(),
        Value::I64(i) => i.to_string(),
        Value::F64(f) => f.to_string(),
        Value::String(s) => s.as_str().to_string(),
        Value::Array(a) => format!("{a:?}"),
        _ => format!("{v:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use haproxy_spoa_hub_plugin_api::RString;

    #[test]
    fn natural_string() {
        let v = SpoeValue::String(RString::from("hello"));
        assert_eq!(natural(&v), Some(Value::String("hello".into())));
    }

    #[test]
    fn natural_ipv4_to_text() {
        let v = SpoeValue::Ipv4([127, 0, 0, 1]);
        assert_eq!(natural(&v), Some(Value::String("127.0.0.1".into())));
    }

    #[test]
    fn natural_uint64_overflow_returns_none() {
        let v = SpoeValue::Uint64(u64::MAX);
        assert_eq!(natural(&v), None);
    }

    #[test]
    fn coerce_int_to_string() {
        let v = SpoeValue::Int64(42);
        assert_eq!(
            coerce(&v, AttributeType::String),
            Some(Value::String("42".into()))
        );
    }

    #[test]
    fn coerce_string_to_bool_true() {
        let v = SpoeValue::String(RString::from("true"));
        assert_eq!(coerce(&v, AttributeType::Bool), Some(Value::Bool(true)));
    }

    #[test]
    fn coerce_invalid_returns_none() {
        let v = SpoeValue::String(RString::from("not-a-number"));
        assert_eq!(coerce(&v, AttributeType::Int), None);
    }

    #[test]
    fn key_value_smoke() {
        let v = SpoeValue::Int64(200);
        let kv = key_value("http.response.status_code", &v, None).unwrap();
        assert_eq!(kv.key.as_str(), "http.response.status_code");
        assert!(matches!(kv.value, Value::I64(200)));
    }
}

//! SCALE Value → JSON conversion preserving the handler JSON contract.
//!
//! This module is generic over the scale-value context type `T`. Call sites in
//! `decode/events.rs` and `decode/extrinsics.rs` instantiate it with
//! `T = u32` via `subxt::dynamic::Value` (the canonical dynamic decode target
//! used by the subxt 0.50 examples). The unit tests below exercise the same
//! code paths with `T = ()` — the converter does not read `T`, so both
//! instantiations share a single test surface.
//!
//! Invariants (violations silently break handlers in `crates/handlers`):
//!   (1) Named composites keep FRAME field names verbatim as JSON object keys.
//!       (`extract_field` in handlers/src/utils.rs searches by name, e.g. "from"/"who".)
//!   (2) 20- and 32-byte unnamed composites of u8-range primitives serialize as
//!       "0x…" hex strings (lowercase, 0x-prefixed). Consumed by `parse_account`
//!       and `parse_hash256`.
//!   (3) u128 and i128 primitives serialize as decimal strings, NOT JSON numbers.
//!       `serde_json::Number` does not carry 128-bit precision and Balances amounts
//!       regularly exceed u64.
//!   (4) `Option::Some`/`None` and `MultiAddress::Id` variants are unwrapped.
//!   (5) Single-element unnamed composites (newtype wrappers) are unwrapped.
//!
//! Difference from the pre-0.50 in-house converter: 64-byte arrays are no longer
//! detected as byte arrays. See spec §4, R5.

use subxt::ext::scale_value::{Composite, Primitive, Value, ValueDef};

pub(crate) fn value_to_json<T>(v: &Value<T>) -> serde_json::Value {
    value_def_to_json(&v.value)
}

pub(crate) fn composite_to_json<T>(c: &Composite<T>) -> serde_json::Value {
    match c {
        Composite::Named(fields) => {
            let map: serde_json::Map<String, serde_json::Value> = fields
                .iter()
                .map(|(name, v)| (name.clone(), value_to_json(v)))
                .collect();
            serde_json::Value::Object(map)
        }
        Composite::Unnamed(values) => {
            if let Some(hex) = try_as_byte_array(values) {
                return serde_json::Value::String(hex);
            }
            if values.len() == 1 {
                return value_to_json(&values[0]);
            }
            serde_json::Value::Array(values.iter().map(value_to_json).collect())
        }
    }
}

fn value_def_to_json<T>(v: &ValueDef<T>) -> serde_json::Value {
    match v {
        ValueDef::Composite(c) => composite_to_json(c),
        ValueDef::Variant(var) => match var.name.as_str() {
            "Some" => unwrap_single(&var.values),
            "None" => serde_json::Value::Null,
            "Id" => unwrap_single(&var.values),
            other => {
                let mut m = serde_json::Map::new();
                m.insert(other.to_string(), composite_to_json(&var.values));
                serde_json::Value::Object(m)
            }
        },
        ValueDef::Primitive(p) => primitive_to_json(p),
        ValueDef::BitSequence(b) => serde_json::Value::String(format!("{:?}", b)),
    }
}

fn unwrap_single<T>(c: &Composite<T>) -> serde_json::Value {
    match c {
        Composite::Unnamed(vs) if vs.len() == 1 => value_to_json(&vs[0]),
        _ => composite_to_json(c),
    }
}

fn try_as_byte_array<T>(values: &[Value<T>]) -> Option<String> {
    let len = values.len();
    if len != 32 && len != 20 {
        return None;
    }
    let mut bytes = Vec::with_capacity(len);
    for v in values {
        let ValueDef::Primitive(Primitive::U128(n)) = &v.value else {
            return None;
        };
        if *n > 255 {
            return None;
        }
        bytes.push(*n as u8);
    }
    Some(format!("0x{}", hex::encode(bytes)))
}

fn primitive_to_json(p: &Primitive) -> serde_json::Value {
    match p {
        Primitive::Bool(b) => serde_json::Value::Bool(*b),
        Primitive::Char(c) => serde_json::Value::String(c.to_string()),
        Primitive::String(s) => serde_json::Value::String(s.clone()),
        Primitive::U128(n) => serde_json::Value::String(n.to_string()),
        Primitive::I128(n) => serde_json::Value::String(n.to_string()),
        Primitive::U256(n) => serde_json::Value::String(format!("{:?}", n)),
        Primitive::I256(n) => serde_json::Value::String(format!("{:?}", n)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use subxt::ext::scale_value::Value;

    // Helper: wrap a primitive in the concrete Value<()> shape. The type parameter
    // on Value is a context carried for richer decoding; `()` is the tests' null context.
    fn u128_val(n: u128) -> Value<()> {
        Value::u128(n)
    }

    #[test]
    fn named_composite_keeps_field_names() {
        // Invariant (1)
        let c: Composite<()> = Composite::Named(vec![
            ("from".to_string(), u128_val(1)),
            ("amount".to_string(), u128_val(1000)),
        ]);
        let json = composite_to_json(&c);
        let obj = json.as_object().expect("object");
        assert!(obj.contains_key("from"));
        assert!(obj.contains_key("amount"));
    }

    #[test]
    fn thirty_two_byte_tuple_becomes_hex_string() {
        // Invariant (2)
        let values: Vec<Value<()>> = (0u8..32).map(|b| u128_val(b as u128)).collect();
        let c = Composite::Unnamed(values);
        let json = composite_to_json(&c);
        let s = json.as_str().expect("string");
        assert!(s.starts_with("0x"));
        assert_eq!(s.len(), 2 + 64); // 0x + 64 hex chars
        assert_eq!(&s[..6], "0x0001");
    }

    #[test]
    fn twenty_byte_tuple_becomes_hex_string() {
        // Invariant (2)
        let values: Vec<Value<()>> = (0u8..20).map(|b| u128_val(b as u128)).collect();
        let c = Composite::Unnamed(values);
        let json = composite_to_json(&c);
        let s = json.as_str().expect("string");
        assert_eq!(s.len(), 2 + 40);
    }

    #[test]
    fn single_element_unnamed_tuple_is_unwrapped() {
        // Invariant (5): newtype wrapper
        let inner = u128_val(42);
        let c = Composite::Unnamed(vec![inner]);
        let json = composite_to_json(&c);
        assert_eq!(json.as_str().unwrap(), "42");
    }

    #[test]
    fn option_some_unwraps_inner() {
        // Invariant (4)
        let inner = u128_val(7);
        let v: Value<()> = Value::variant("Some", Composite::Unnamed(vec![inner]));
        let json = value_to_json(&v);
        assert_eq!(json.as_str().unwrap(), "7");
    }

    #[test]
    fn option_none_is_null() {
        // Invariant (4)
        let v: Value<()> = Value::variant("None", Composite::Unnamed(vec![]));
        let json = value_to_json(&v);
        assert!(json.is_null());
    }

    #[test]
    fn multi_address_id_unwraps() {
        // Invariant (4)
        let account: Vec<Value<()>> = (0u8..32).map(|b| u128_val(b as u128)).collect();
        let inner: Value<()> = Value::unnamed_composite(account);
        let v: Value<()> = Value::variant("Id", Composite::Unnamed(vec![inner]));
        let json = value_to_json(&v);
        let s = json.as_str().unwrap();
        assert!(s.starts_with("0x"));
    }

    #[test]
    fn other_variants_nest_in_object() {
        let inner = u128_val(1);
        let v: Value<()> = Value::variant("MyVariant", Composite::Unnamed(vec![inner]));
        let json = value_to_json(&v);
        let obj = json.as_object().unwrap();
        assert!(obj.contains_key("MyVariant"));
        // Inner value is a single-element unnamed composite, so it unwraps via
        // composite_to_json -> single-element -> primitive -> decimal string "1".
        assert_eq!(obj.get("MyVariant").unwrap().as_str(), Some("1"));
    }

    #[test]
    fn u128_max_serializes_as_decimal_string_without_precision_loss() {
        // Invariant (3)
        let v = u128_val(u128::MAX);
        let json = value_to_json(&v);
        let s = json.as_str().unwrap();
        assert_eq!(s, "340282366920938463463374607431768211455");
        assert_eq!(s.parse::<u128>().unwrap(), u128::MAX);
    }

    #[test]
    fn thirty_two_element_tuple_with_values_over_u8_is_not_a_byte_array() {
        // Regression guard: something that looks like a 32-tuple but isn't all bytes
        // must NOT be smashed into a hex string.
        let values: Vec<Value<()>> = (0u128..32).map(|_| u128_val(256)).collect();
        let c = Composite::Unnamed(values);
        let json = composite_to_json(&c);
        assert!(json.is_array(), "expected array, got {}", json);
    }

    #[test]
    fn named_composite_with_u128_above_u64_max_serializes_as_decimal_string() {
        // Invariant (3) end-to-end: a Balances-shaped event (Named composite
        // with an `amount` field carrying u128 > u64::MAX) must produce a
        // *string* JSON value, not a Number. Otherwise `handlers::parse_amount`
        // takes the `Number` branch, calls `n.as_u64()`, and silently clips
        // the amount — or returns None for values that don't fit as u64.
        let amount = (u64::MAX as u128) + 1;
        let c: Composite<()> = Composite::Named(vec![
            ("from".to_string(), u128_val(1)),
            ("to".to_string(), u128_val(2)),
            ("amount".to_string(), u128_val(amount)),
        ]);
        let json = composite_to_json(&c);
        let obj = json.as_object().expect("object");
        let amount_json = obj.get("amount").expect("amount field");
        assert!(
            amount_json.is_string(),
            "amount must be a JSON string, got {amount_json}"
        );
        let parsed: u128 = amount_json
            .as_str()
            .unwrap()
            .parse()
            .expect("parses as u128");
        assert_eq!(parsed, amount);
    }

    #[test]
    fn variant_with_named_payload_preserves_field_names() {
        // FRAME Event shape: `MyEvent::Transfer { from, to, amount }` decodes
        // into `Variant { name: "Transfer", values: Named([...]) }`. The outer
        // variant wraps in `{"Transfer": {...}}`, and the inner named composite
        // must keep its field names verbatim (invariant (1)).
        let inner: Composite<()> = Composite::Named(vec![
            ("from".to_string(), u128_val(1)),
            ("to".to_string(), u128_val(2)),
            ("amount".to_string(), u128_val(1000)),
        ]);
        let v: Value<()> = Value::variant("Transfer", inner);
        let json = value_to_json(&v);
        let outer = json.as_object().expect("object");
        let transfer = outer
            .get("Transfer")
            .and_then(|v| v.as_object())
            .expect("Transfer object");
        assert!(transfer.contains_key("from"));
        assert!(transfer.contains_key("to"));
        assert!(transfer.contains_key("amount"));
        assert_eq!(transfer.get("amount").unwrap().as_str(), Some("1000"));
    }

    #[test]
    fn option_some_of_named_struct_unwraps_and_preserves_fields() {
        // `Option<Something { free: u128 }>` in the Some arm: exercises the
        // unwrap_single → Composite::Unnamed(1) → inner Composite::Named path.
        let inner_struct: Value<()> =
            Value::named_composite(vec![("free".to_string(), u128_val(42))]);
        let v: Value<()> = Value::variant("Some", Composite::Unnamed(vec![inner_struct]));
        let json = value_to_json(&v);
        let obj = json.as_object().expect("object");
        assert_eq!(obj.get("free").unwrap().as_str(), Some("42"));
    }

    #[test]
    fn sixty_four_element_byte_range_tuple_is_array_not_hex() {
        // Deliberate behavior change from pre-0.50 converter (spec §4 R5):
        // 64-byte unnamed composites (e.g., Signature) are NOT interpreted as
        // byte arrays. This guards against regressions that generalize the
        // byte-array detector back to accept 64-byte tuples.
        let values: Vec<Value<()>> = (0u8..64).map(|b| u128_val(b as u128)).collect();
        let c = Composite::Unnamed(values);
        let json = composite_to_json(&c);
        assert!(
            json.is_array(),
            "64-byte tuple must not be hex-flattened, got {}",
            json
        );
        assert_eq!(json.as_array().unwrap().len(), 64);
    }
}

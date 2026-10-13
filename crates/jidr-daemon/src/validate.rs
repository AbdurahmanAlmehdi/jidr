//! Params validation against a command's JSON Schema, before any lock is
//! taken or handler runs: object shape, unknown keys, required keys and
//! primitive types. Handlers still deserialize strictly; this makes the error
//! uniform (`usage`, -32602, naming the key) for every provider.

use jidr_core::{JidrError, JidrResult};
use serde_json::Value;

/// `params` (null = `{}`) against `schema` (schemars output).
pub fn validate(schema: &Value, params: &Value) -> JidrResult<()> {
    let empty = Value::Object(Default::default());
    let params = if params.is_null() { &empty } else { params };
    let Some(obj) = params.as_object() else {
        return Err(JidrError::usage("invalid params: expected an object"));
    };
    let props = schema.get("properties").and_then(Value::as_object);
    if schema.get("additionalProperties") == Some(&Value::Bool(false)) {
        for key in obj.keys() {
            if !props.is_some_and(|p| p.contains_key(key)) {
                return Err(JidrError::usage(format!(
                    "invalid params: unknown field `{key}`"
                )));
            }
        }
    }
    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        for key in required.iter().filter_map(Value::as_str) {
            if !obj.contains_key(key) {
                return Err(JidrError::usage(format!(
                    "invalid params: missing field `{key}`"
                )));
            }
        }
    }
    if let Some(props) = props {
        for (key, value) in obj {
            if let Some(prop) = props.get(key)
                && !type_matches(prop, value)
            {
                return Err(JidrError::usage(format!(
                    "invalid params: `{key}` must be {}",
                    describe(prop)
                )));
            }
        }
    }
    Ok(())
}

/// The property's declared JSON types; `None` when it uses `$ref`/`anyOf`/…
/// (left to the handler's own deserialization).
fn declared_types(prop: &Value) -> Option<Vec<&str>> {
    match prop.get("type")? {
        Value::String(t) => Some(vec![t.as_str()]),
        Value::Array(ts) => Some(ts.iter().filter_map(Value::as_str).collect()),
        _ => None,
    }
}

fn type_matches(prop: &Value, v: &Value) -> bool {
    let Some(types) = declared_types(prop) else {
        return true;
    };
    types.iter().any(|t| match *t {
        "string" => v.is_string(),
        "boolean" => v.is_boolean(),
        "integer" => v.is_i64() || v.is_u64(),
        "number" => v.is_number(),
        "array" => v.is_array(),
        "object" => v.is_object(),
        "null" => v.is_null(),
        _ => true,
    })
}

fn describe(prop: &Value) -> String {
    declared_types(prop)
        .map(|t| t.join(" or "))
        .unwrap_or_else(|| "valid".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema() -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["device"],
            "properties": {
                "device": { "type": "string" },
                "cold_boot": { "type": "boolean" },
                "lines": { "type": ["integer", "null"], "format": "uint32" },
                "mode": { "$ref": "#/$defs/RootMode" }
            }
        })
    }

    #[test]
    fn accepts_valid_params() {
        validate(&schema(), &json!({ "device": "a" })).unwrap();
        validate(
            &schema(),
            &json!({ "device": "a", "cold_boot": true, "lines": null, "mode": "adb" }),
        )
        .unwrap();
        validate(&json!({ "type": "object", "properties": {} }), &Value::Null).unwrap();
    }

    #[test]
    fn rejects_with_the_key_named() {
        let cases = [
            (json!({ "device": "a", "colld": 1 }), "colld"),
            (json!({}), "device"),
            (json!({ "device": 5 }), "device"),
            (json!({ "device": "a", "cold_boot": "yes" }), "cold_boot"),
            (json!({ "device": "a", "lines": 1.5 }), "lines"),
            (json!([1, 2]), "object"),
        ];
        for (params, key) in cases {
            let e = validate(&schema(), &params).unwrap_err();
            assert_eq!(e.code, jidr_core::ErrorCode::Usage);
            assert!(e.message.contains(key), "{params} → {e}");
        }
    }
}

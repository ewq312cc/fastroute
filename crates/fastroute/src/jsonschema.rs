// A small JSON Schema interpreter for the published schemas in `docs/schema/`, so that the
// schema file itself is what is enforced at run time (and by the tests), not a hand-written copy.
//
// Subset: local `$ref`, `type` (also a list), `enum`, `const`, `required`, `properties`,
// `additionalProperties` (`false` or a schema), `items`, `minItems`, `minimum`, `if`/`then`.
// Everything else in a schema (titles, descriptions, `$defs`, ...) is ignored.
//
// This file is also compiled into the integration tests (`include!`), so it must not use
// anything but `serde_json`.

use serde_json::Value;

/// Every way `value` does not match `schema`, as `path: message` lines (empty = valid).
pub fn validate(schema: &Value, value: &Value) -> Vec<String> {
    let mut errors = Vec::new();
    check(schema, schema, value, "$", &mut errors);
    errors
}

fn resolve<'a>(root: &'a Value, reference: &str) -> Option<&'a Value> {
    root.pointer(reference.strip_prefix('#')?)
}

fn type_matches(t: &str, v: &Value) -> bool {
    match t {
        "object" => v.is_object(),
        "array" => v.is_array(),
        "string" => v.is_string(),
        "number" => v.is_number(),
        "integer" => v.is_i64() || v.is_u64(),
        "boolean" => v.is_boolean(),
        "null" => v.is_null(),
        _ => false,
    }
}

fn check(root: &Value, schema: &Value, value: &Value, path: &str, errors: &mut Vec<String>) {
    if let Some(r) = schema.get("$ref").and_then(Value::as_str) {
        match resolve(root, r) {
            Some(target) => check(root, target, value, path, errors),
            None => errors.push(format!("{path}: the schema refers to {r}, which does not exist")),
        }
        return;
    }
    if let Some(types) = schema.get("type") {
        let list: Vec<&str> = match types {
            Value::String(t) => vec![t.as_str()],
            Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
            _ => vec![],
        };
        if !list.iter().any(|t| type_matches(t, value)) {
            errors.push(format!("{path}: expected {}, got {}", list.join(" or "), brief(value)));
            return;
        }
    }
    if let Some(c) = schema.get("const") {
        if c != value {
            errors.push(format!("{path}: expected the constant {c}, got {}", brief(value)));
        }
    }
    if let Some(e) = schema.get("enum").and_then(Value::as_array) {
        if !e.contains(value) {
            let allowed: Vec<String> = e.iter().map(Value::to_string).collect();
            errors.push(format!("{path}: {} is not one of {}", brief(value), allowed.join(", ")));
        }
    }
    if let (Some(min), Some(v)) = (schema.get("minimum").and_then(Value::as_f64), value.as_f64()) {
        if v < min {
            errors.push(format!("{path}: {v} is below the minimum {min}"));
        }
    }
    if let Some(obj) = value.as_object() {
        for r in schema.get("required").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
            if !obj.contains_key(r) {
                errors.push(format!("{path}: missing required property '{r}'"));
            }
        }
        let props = schema.get("properties").and_then(Value::as_object);
        for (k, v) in obj {
            let child = format!("{path}.{k}");
            match props.and_then(|p| p.get(k)) {
                Some(s) => check(root, s, v, &child, errors),
                None => match schema.get("additionalProperties") {
                    Some(Value::Bool(false)) => errors.push(format!("{path}: unexpected property '{k}'")),
                    Some(s @ Value::Object(_)) => check(root, s, v, &child, errors),
                    _ => {}
                },
            }
        }
    }
    if let (Some(cond), Some(then)) = (schema.get("if"), schema.get("then")) {
        let mut inner = Vec::new();
        check(root, cond, value, path, &mut inner);
        if inner.is_empty() {
            check(root, then, value, path, errors);
        }
    }
    if let (Some(arr), Some(items)) = (value.as_array(), schema.get("items")) {
        if let Some(n) = schema.get("minItems").and_then(Value::as_u64) {
            if (arr.len() as u64) < n {
                errors.push(format!("{path}: needs at least {n} items, has {}", arr.len()));
            }
        }
        for (i, v) in arr.iter().enumerate() {
            check(root, items, v, &format!("{path}[{i}]"), errors);
        }
    }
}

fn brief(v: &Value) -> String {
    let s = v.to_string();
    if s.chars().count() > 60 {
        format!("{}...", s.chars().take(57).collect::<String>())
    } else {
        s
    }
}

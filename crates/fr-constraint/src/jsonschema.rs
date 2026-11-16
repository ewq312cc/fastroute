//! A small JSON Schema interpreter, so that the published schema files are what is enforced (not
//! a hand-written copy of them). Subset: local `$ref`, `type`, `enum`, `const`, `required`,
//! `properties`, `patternProperties`, `additionalProperties`, `items`, `minItems`, `maxItems`,
//! `minLength`, `maxLength`, `pattern`, `minimum`, `maximum`, `if`/`then`/`else`, `allOf`,
//! `anyOf`, `oneOf` and `not`. Everything else (titles, descriptions, `$defs`, formats) is ignored.
//! The tests cross-check it with the reference implementation on the same files.

use regex::Regex;
use serde_json::Value;

/// One place where a value does not match its schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaError {
    /// `$.rules[3].params.pins[0]`
    pub path: String,
    pub message: String,
}

/// Every way `value` does not match `schema` (empty = valid).
pub fn validate(schema: &Value, value: &Value) -> Vec<SchemaError> {
    let mut errors = Vec::new();
    check(schema, schema, value, "$", &mut errors);
    errors
}

fn err(errors: &mut Vec<SchemaError>, path: &str, message: String) {
    errors.push(SchemaError { path: path.to_string(), message });
}

fn type_matches(t: &str, v: &Value) -> bool {
    match t {
        "object" => v.is_object(),
        "array" => v.is_array(),
        "string" => v.is_string(),
        "number" => v.is_number(),
        "integer" => v.is_i64() || v.is_u64() || v.as_f64().is_some_and(|f| f.fract() == 0.0),
        "boolean" => v.is_boolean(),
        "null" => v.is_null(),
        _ => false,
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

fn is_valid(root: &Value, schema: &Value, value: &Value) -> bool {
    let mut scratch = Vec::new();
    check(root, schema, value, "$", &mut scratch);
    scratch.is_empty()
}

fn child(path: &str, key: &str) -> String {
    format!("{path}.{key}")
}

fn check(root: &Value, schema: &Value, value: &Value, path: &str, errors: &mut Vec<SchemaError>) {
    let obj = match schema {
        Value::Bool(true) => return,
        Value::Bool(false) => return err(errors, path, "no value is allowed here".into()),
        Value::Object(o) => o,
        _ => return,
    };
    if let Some(r) = obj.get("$ref").and_then(Value::as_str) {
        match r.strip_prefix('#').and_then(|p| root.pointer(p)) {
            Some(target) => check(root, target, value, path, errors),
            None => err(errors, path, format!("the schema refers to {r}, which does not exist")),
        }
        return;
    }
    if let Some(types) = obj.get("type") {
        let list: Vec<&str> = match types {
            Value::String(t) => vec![t.as_str()],
            Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
            _ => vec![],
        };
        if !list.iter().any(|t| type_matches(t, value)) {
            err(errors, path, format!("expected {}, got {}", list.join(" or "), brief(value)));
            return;
        }
    }
    if let Some(c) = obj.get("const") {
        if c != value {
            err(errors, path, format!("expected the constant {c}, got {}", brief(value)));
        }
    }
    if let Some(Value::Array(allowed)) = obj.get("enum") {
        if !allowed.contains(value) {
            let list: Vec<String> = allowed.iter().map(|a| a.to_string()).collect();
            err(errors, path, format!("{} is not one of {}", brief(value), list.join(", ")));
        }
    }
    if let Some(s) = value.as_str() {
        let n = s.chars().count() as u64;
        if let Some(min) = obj.get("minLength").and_then(Value::as_u64) {
            if n < min {
                err(errors, path, format!("must have at least {min} characters"));
            }
        }
        if let Some(max) = obj.get("maxLength").and_then(Value::as_u64) {
            if n > max {
                err(errors, path, format!("must have at most {max} characters, has {n}"));
            }
        }
        if let Some(p) = obj.get("pattern").and_then(Value::as_str) {
            match Regex::new(p) {
                Ok(re) if !re.is_match(s) => err(errors, path, format!("{} does not match the pattern {p}", brief(value))),
                Err(e) => err(errors, path, format!("the schema has an invalid pattern {p}: {e}")),
                _ => {}
            }
        }
    }
    if let Some(x) = value.as_f64().filter(|_| value.is_number()) {
        if let Some(min) = obj.get("minimum").and_then(Value::as_f64) {
            if x < min {
                err(errors, path, format!("{x} is below the minimum {min}"));
            }
        }
        if let Some(max) = obj.get("maximum").and_then(Value::as_f64) {
            if x > max {
                err(errors, path, format!("{x} is above the maximum {max}"));
            }
        }
        if let Some(min) = obj.get("exclusiveMinimum").and_then(Value::as_f64) {
            if x <= min {
                err(errors, path, format!("{x} is not above the exclusive minimum {min}"));
            }
        }
        if let Some(max) = obj.get("exclusiveMaximum").and_then(Value::as_f64) {
            if x >= max {
                err(errors, path, format!("{x} is not below the exclusive maximum {max}"));
            }
        }
    }
    if let Some(map) = value.as_object() {
        if let Some(Value::Array(req)) = obj.get("required") {
            for r in req.iter().filter_map(Value::as_str) {
                if !map.contains_key(r) {
                    err(errors, path, format!("missing required property '{r}'"));
                }
            }
        }
        let props = obj.get("properties").and_then(Value::as_object);
        let patterns: Vec<(Regex, &Value)> = obj
            .get("patternProperties")
            .and_then(Value::as_object)
            .map(|m| m.iter().filter_map(|(k, v)| Regex::new(k).ok().map(|re| (re, v))).collect())
            .unwrap_or_default();
        for (k, v) in map {
            let mut known = false;
            if let Some(sub) = props.and_then(|p| p.get(k)) {
                known = true;
                check(root, sub, v, &child(path, k), errors);
            }
            for (re, sub) in &patterns {
                if re.is_match(k) {
                    known = true;
                    check(root, sub, v, &child(path, k), errors);
                }
            }
            if !known {
                match obj.get("additionalProperties") {
                    Some(Value::Bool(false)) => err(errors, &child(path, k), format!("unexpected property '{k}'")),
                    Some(sub @ Value::Object(_)) => check(root, sub, v, &child(path, k), errors),
                    _ => {}
                }
            }
        }
    }
    if let Some(items) = value.as_array() {
        if let Some(min) = obj.get("minItems").and_then(Value::as_u64) {
            if (items.len() as u64) < min {
                err(errors, path, format!("needs at least {min} item(s), has {}", items.len()));
            }
        }
        if let Some(max) = obj.get("maxItems").and_then(Value::as_u64) {
            if (items.len() as u64) > max {
                err(errors, path, format!("allows at most {max} item(s), has {}", items.len()));
            }
        }
        if let Some(sub) = obj.get("items") {
            for (i, v) in items.iter().enumerate() {
                check(root, sub, v, &format!("{path}[{i}]"), errors);
            }
        }
    }
    if let Some(Value::Array(all)) = obj.get("allOf") {
        for s in all {
            check(root, s, value, path, errors);
        }
    }
    if let Some(Value::Array(any)) = obj.get("anyOf") {
        if !any.iter().any(|s| is_valid(root, s, value)) {
            let needs: Vec<String> = any
                .iter()
                .filter_map(|s| s.get("required").and_then(Value::as_array))
                .flat_map(|r| r.iter().filter_map(Value::as_str).map(str::to_string))
                .collect();
            if !needs.is_empty() && needs.len() == any.len() {
                err(errors, path, format!("needs at least one of: {}", needs.join(", ")));
            } else {
                err(errors, path, "matches none of the allowed alternatives".into());
            }
        }
    }
    if let Some(Value::Array(one)) = obj.get("oneOf") {
        let n = one.iter().filter(|s| is_valid(root, s, value)).count();
        if n != 1 {
            err(errors, path, format!("must match exactly one alternative, matches {n}"));
        }
    }
    if let Some(sub) = obj.get("not") {
        if is_valid(root, sub, value) {
            err(errors, path, "matches something that is not allowed here".into());
        }
    }
    if let Some(cond) = obj.get("if") {
        if is_valid(root, cond, value) {
            if let Some(then) = obj.get("then") {
                check(root, then, value, path, errors);
            }
        } else if let Some(otherwise) = obj.get("else") {
            check(root, otherwise, value, path, errors);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_subset_works() {
        let schema = json!({
            "type": "object", "required": ["a"], "additionalProperties": false,
            "patternProperties": {"^x_": {}},
            "properties": {"a": {"type": "integer", "minimum": 1}, "b": {"enum": ["u", "v"]}, "c": {"type": "array", "minItems": 1, "items": {"type": "string", "pattern": "^S"}}},
            "allOf": [{"if": {"properties": {"b": {"const": "u"}}, "required": ["b"]}, "then": {"required": ["c"]}}],
            "anyOf": [{"required": ["a"]}, {"required": ["b"]}]
        });
        assert!(validate(&schema, &json!({"a": 1, "x_note": 5})).is_empty());
        let e = validate(&schema, &json!({"a": 0, "b": "u", "z": 1, "c": ["T"]}));
        let text: Vec<String> = e.iter().map(|e| format!("{}: {}", e.path, e.message)).collect();
        assert!(text.iter().any(|t| t.contains("$.a") && t.contains("minimum")), "{text:?}");
        assert!(text.iter().any(|t| t.contains("$.z") && t.contains("unexpected")), "{text:?}");
        assert!(text.iter().any(|t| t.contains("$.c[0]") && t.contains("pattern")), "{text:?}");
        let e = validate(&schema, &json!({"a": 1, "b": "u"}));
        assert!(e.iter().any(|e| e.message.contains("missing required property 'c'")), "{e:?}");
        let e = validate(&schema, &json!({"b": "v"}));
        assert!(e.iter().any(|e| e.message.contains("missing required property 'a'")), "{e:?}");
        let e = validate(&json!({"not": {"enum": [1]}}), &json!(1));
        assert_eq!(e.len(), 1);
    }
}

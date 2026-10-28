//! The Python habits the ported actions keep, so their notes and prompts read exactly like the text apps': truthiness,
//! str() of a JSON value, shlex quoting, subscripts that fail like a KeyError, and a JSON object read in file order.

use serde_json::Value;

pub type Res<T = ()> = Result<T, String>;

pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// What an f-string prints for the value: None for null, True/False, 2.0 for a whole float.
pub fn text(v: &Value) -> String {
    match v {
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::String(s) => s.clone(),
        Value::Number(n) => match (n.as_i64(), n.as_u64(), n.as_f64()) {
            (Some(i), _, _) => i.to_string(),
            (_, Some(u), _) => u.to_string(),
            (_, _, Some(f)) if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e16 => {
                format!("{f:.1}")
            }
            (_, _, Some(f)) => f.to_string(),
            _ => n.to_string(),
        },
        other => other.to_string(),
    }
}

/// `value[key]`: a missing key fails like Python's KeyError, a null like a None subscript.
pub fn key<'a>(v: &'a Value, k: &str) -> Res<&'a Value> {
    match v {
        Value::Object(map) => map.get(k).ok_or_else(|| format!("'{k}'")),
        Value::Null => Err("'NoneType' object is not subscriptable".into()),
        _ => Err(format!("cannot read '{k}' of {v}")),
    }
}

/// `str(value[key])`.
pub fn s(v: &Value, k: &str) -> Res<String> {
    key(v, k).map(text)
}

/// `value.get(key)`: null when missing.
pub fn get<'a>(v: &'a Value, k: &str) -> &'a Value {
    static NULL: Value = Value::Null;
    v.get(k).unwrap_or(&NULL)
}

/// `value.get(key) or default` as text.
pub fn or(v: &Value, k: &str, default: &str) -> String {
    let found = get(v, k);
    if truthy(found) { text(found) } else { default.into() }
}

/// A list the way `x or []` reads it.
pub fn list(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or_default()
}

/// shlex.quote.
pub fn quote(s: &str) -> String {
    if s.is_empty() {
        return "''".into();
    }
    if s.chars().all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c)) {
        return s.into();
    }
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}

/// shlex.join.
pub fn join(words: &[String]) -> String {
    words.iter().map(|w| quote(w)).collect::<Vec<_>>().join(" ")
}

/// The first `n` characters, like `text[:n]`.
pub fn cut(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// A JSON object's entries in the order the file lists them, as a Python dict keeps them.
pub fn ordered(text: &str) -> Vec<(String, Value)> {
    struct Entries(Vec<(String, Value)>);
    impl<'de> serde::Deserialize<'de> for Entries {
        fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            struct Visit;
            impl<'de> serde::de::Visitor<'de> for Visit {
                type Value = Entries;
                fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                    f.write_str("an object")
                }
                fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Entries, A::Error> {
                    let mut out = Vec::new();
                    while let Some(entry) = map.next_entry::<String, Value>()? {
                        out.push(entry);
                    }
                    Ok(Entries(out))
                }
            }
            d.deserialize_map(Visit)
        }
    }
    serde_json::from_str::<Entries>(text).map(|e| e.0).unwrap_or_default()
}

pub fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

pub fn now_ms() -> i64 {
    (now() * 1000.0) as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn text_prints_like_python() {
        assert_eq!(text(&json!(null)), "None");
        assert_eq!(text(&json!(2)), "2");
        assert_eq!(text(&json!(2.0)), "2.0");
        assert_eq!(text(&json!(2.5)), "2.5");
        assert_eq!(text(&json!("x")), "x");
    }

    #[test]
    fn quote_like_shlex() {
        assert_eq!(quote(""), "''");
        assert_eq!(quote("a/b-c.d"), "a/b-c.d");
        assert_eq!(quote("it's"), "'it'\"'\"'s'");
        assert_eq!(join(&["claude".into(), "/code-review 12".into()]), "claude '/code-review 12'");
    }

    #[test]
    fn truthiness() {
        assert!(!truthy(&json!({})));
        assert!(!truthy(&json!("")));
        assert!(!truthy(&json!(0)));
        assert!(truthy(&json!({"a": 1})));
    }

    #[test]
    fn ordered_keeps_file_order() {
        let keys: Vec<String> = ordered(r#"{"b": 1, "a": 2, "c": 3}"#).into_iter().map(|(k, _)| k).collect();
        assert_eq!(keys, ["b", "a", "c"]);
    }

    #[test]
    fn key_errors_read_like_python() {
        assert_eq!(key(&json!({}), "url"), Err("'url'".to_string()));
    }
}

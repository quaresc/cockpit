//! A connector's mapping, as the Claude plugin reads it (plugin/hooks/platform.ts): a tool's arguments filled from a
//! template, and values read from its answer at dotted paths.

use serde_json::{Map, Value};

/// The value at a dotted path ("status.health", "urls.0.url"); "a|b" takes the first path that has one.
pub fn pick<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    if path.is_empty() {
        return None;
    }
    path.split('|').find_map(|option| {
        let found = option.split('.').try_fold(value, |node, key| match node {
            Value::Object(map) => map.get(key),
            Value::Array(items) => key.parse::<usize>().ok().and_then(|i| items.get(i)),
            _ => None,
        })?;
        (!found.is_null()).then_some(found)
    })
}

/// A value as text, as the plugin's `text` does: strings as they are, other values in their JSON form.
pub fn text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

/// The list at a dotted path, where a `[]` segment flattens a list on the way; an empty path is the answer itself.
pub fn list_at<'a>(value: &'a Value, path: &str) -> Vec<&'a Value> {
    let list = |v: Option<&'a Value>| -> Vec<&'a Value> { v.and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default() };
    if path.is_empty() {
        return list(Some(value));
    }
    let nodes = path.split('.').fold(vec![Some(value)], |found, key| match key.strip_suffix("[]") {
        Some(key) => found.into_iter().flat_map(|n| list(n.and_then(|n| pick(n, key)))).map(Some).collect(),
        None => found.into_iter().map(|n| n.and_then(|n| pick(n, key))).collect(),
    });
    nodes.into_iter().flat_map(list).collect()
}

/// The arguments with their "{name}" placeholders filled: a whole-string placeholder keeps the value's type, and one
/// without a value is `None`, so its argument is left out.
pub fn fill_args(template: &Value, values: &Map<String, Value>) -> Option<Value> {
    match template {
        Value::String(s) => {
            if let Some(name) = s.strip_prefix('{').and_then(|r| r.strip_suffix('}')).filter(|n| is_word(n)) {
                return values.get(name).cloned();
            }
            Some(Value::String(replace_placeholders(s, values)))
        }
        Value::Array(items) => Some(Value::Array(items.iter().map(|t| fill_args(t, values).unwrap_or(Value::Null)).collect())),
        Value::Object(map) => Some(Value::Object(map.iter().filter_map(|(k, v)| Some((k.clone(), fill_args(v, values)?))).collect())),
        other => Some(other.clone()),
    }
}

/// A "{name}" template filled from `values`, as an argument's text is.
pub fn fill(template: &str, values: &Map<String, Value>) -> String {
    replace_placeholders(template, values)
}

/// The values a "{name}" template reads out of a text ("github.com/{owner}/{repo}/issues/{number}" over an issue's
/// URL, "{owner}/{repo}#{number}" over its id); `None` when a literal piece of the template is missing. A value of
/// digits becomes a number, for an argument that takes one.
pub fn capture(template: &str, text: &str) -> Option<Map<String, Value>> {
    let (mut values, mut rest, mut left) = (Map::new(), template, text);
    while let Some(open) = rest.find('{') {
        let close = rest[open..].find('}').map(|c| open + c)?;
        let name = &rest[open + 1..close];
        if !is_word(name) {
            return None;
        }
        let before = &rest[..open];
        left = &left[left.find(before)? + before.len()..];
        rest = &rest[close + 1..];
        let until = rest.split('{').next().unwrap_or_default();
        let end = if until.is_empty() { left.len() } else { left.find(until)? };
        let (value, after) = left.split_at(end);
        if value.is_empty() {
            return None;
        }
        values.insert(name.to_string(), value.parse::<u64>().map(Value::from).unwrap_or_else(|_| Value::String(value.to_string())));
        left = after;
    }
    Some(values)
}

fn is_word(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn replace_placeholders(s: &str, values: &Map<String, Value>) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}').map(|c| open + c) else { break };
        let name = &rest[open + 1..close];
        out.push_str(&rest[..open]);
        if is_word(name) {
            out.push_str(&text(values.get(name)).unwrap_or_default());
        } else {
            out.push_str(&rest[open..=close]);
        }
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn picks_paths_with_fallbacks_and_indexes() {
        let v = json!({"status": {"health": "ok"}, "urls": [{"url": "u"}], "empty": null});
        assert_eq!(pick(&v, "status.health"), Some(&json!("ok")));
        assert_eq!(pick(&v, "urls.0.url"), Some(&json!("u")));
        assert_eq!(pick(&v, "empty|status.health"), Some(&json!("ok")));
        assert_eq!(pick(&v, "missing"), None);
        assert_eq!(text(pick(&v, "urls.0")), Some("{\"url\":\"u\"}".into()));
    }

    #[test]
    fn lists_flatten_on_brackets() {
        let v = json!({"issues": [1, 2], "runs": [{"steps": [1, 2]}, {"steps": [3]}]});
        assert_eq!(list_at(&v, "issues").len(), 2);
        assert_eq!(list_at(&v, "runs[].steps"), vec![&json!(1), &json!(2), &json!(3)]);
        assert_eq!(list_at(&json!([1]), "").len(), 1);
        assert!(list_at(&v, "missing").is_empty());
    }

    #[test]
    fn a_template_reads_its_parts_back_out_of_a_text() {
        let parts = |t, text| capture(t, text).map(Value::Object);
        let url = "https://github.com/acme/api/issues/22";
        assert_eq!(parts("github.com/{owner}/{repo}/issues/{number}", url), Some(json!({"owner": "acme", "repo": "api", "number": 22})));
        assert_eq!(parts("{owner}/{repo}#{number}", "acme/api#22"), Some(json!({"owner": "acme", "repo": "api", "number": 22})));
        assert_eq!(parts("github.com/{owner}/{repo}/issues/{number}", "https://gitlab.com/acme/api/-/issues/22"), None);
        assert_eq!(parts("{owner}/{repo}#{number}", "ENG-1"), None);
        let values = capture("{owner}/{repo}#{number}", "acme/api#22").unwrap();
        assert_eq!(fill("{owner}/{repo}#{number}", &values), "acme/api#22");
    }

    #[test]
    fn fills_placeholders_keeping_whole_values_typed() {
        let values = json!({"ticket": "ENG-1", "n": 3}).as_object().unwrap().clone();
        let args = json!({"id": "{ticket}", "limit": "{n}", "q": "key = {ticket}", "page": "{cursor}", "keep": true});
        assert_eq!(fill_args(&args, &values), Some(json!({"id": "ENG-1", "limit": 3, "q": "key = ENG-1", "keep": true})));
    }
}

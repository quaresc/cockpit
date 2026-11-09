//! A source's tools by verb (listMine, listRuns, listEvents…): a server called through the section's mapping, or
//! answers given by hand in tests.

use serde_json::Value;

use crate::client::{Connection, call_mapped};

#[allow(async_fn_in_trait)]
pub trait Tools {
    /// The answer of the tool mapped to `verb`, its placeholders filled from `values`.
    async fn call(&self, verb: &str, values: &Value) -> Result<Value, String>;
    /// Whether a tool is mapped to `verb`.
    fn has(&self, verb: &str) -> bool;
    /// A tool called by its own name, its answer's text as it is (servers whose tools cockpit names itself).
    async fn text(&self, tool: &str, args: &Value) -> Result<String, String>;
}

/// A server and the config section that maps its tools.
pub struct Mapped<'a> {
    pub connection: &'a Connection,
    pub section: &'a Value,
}

impl Tools for Mapped<'_> {
    async fn call(&self, verb: &str, values: &Value) -> Result<Value, String> {
        call_mapped(self.connection, self.section, verb, values).await
    }

    fn has(&self, verb: &str) -> bool {
        self.section["tools"][verb]["tool"].as_str().is_some_and(|t| !t.is_empty())
    }

    async fn text(&self, tool: &str, args: &Value) -> Result<String, String> {
        self.connection.call_text(tool, args.as_object().cloned().unwrap_or_default()).await
    }
}

/// Answers by verb (or by tool name for `text`), each a list taken in turn (the last one repeats), and the calls made,
/// for tests; a `text` answer that is a JSON string is given as that string, and `{"_error": "…"}` fails the call.
#[derive(Default)]
pub struct Fake {
    pub answers: std::collections::HashMap<String, Vec<Value>>,
    pub calls: std::cell::RefCell<Vec<(String, Value)>>,
}

impl Fake {
    pub fn with(mut self, verb: &str, answer: Value) -> Self {
        self.answers.entry(verb.to_string()).or_default().push(answer);
        self
    }
}

impl Tools for Fake {
    async fn call(&self, verb: &str, values: &Value) -> Result<Value, String> {
        let n = self.calls.borrow().iter().filter(|(v, _)| v == verb).count();
        self.calls.borrow_mut().push((verb.to_string(), values.clone()));
        let answers = self.answers.get(verb).ok_or_else(|| format!("no tool mapped for {verb}"))?;
        let answer = answers.get(n).or(answers.last()).cloned().unwrap_or(Value::Null);
        match answer["_error"].as_str() {
            Some(error) => Err(error.to_string()),
            None => Ok(answer),
        }
    }

    fn has(&self, verb: &str) -> bool {
        self.answers.contains_key(verb)
    }

    async fn text(&self, tool: &str, args: &Value) -> Result<String, String> {
        Ok(match self.call(tool, args).await? {
            Value::String(s) => s,
            other => other.to_string(),
        })
    }
}

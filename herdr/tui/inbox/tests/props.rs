//! Whatever shape the caches take, the model builds and the app draws without panicking.

mod common;

use std::collections::HashMap;

use cockpit_core::testing::render;
use cockpit_inbox::app::App;
use cockpit_inbox::inputs::Inputs;
use cockpit_inbox::sessions::Local;
use proptest::prelude::*;
use serde_json::{Map, Value, json};

/// Any JSON value, a few levels deep.
fn any_json() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        any::<f64>().prop_filter("finite", |f| f.is_finite()).prop_map(Value::from),
        "[a-zA-Z0-9 #:/._-]{0,12}".prop_map(Value::from),
        prop_oneof![Just("2026-10-27T15:53:26Z"), Just("FAILURE"), Just("SUCCESS"), Just("APPROVED"), Just("started"), Just("ENG-1"), Just("eng-1-x"), Just("api")].prop_map(Value::from),
    ];
    leaf.prop_recursive(3, 24, 6, |inner| {
        prop_oneof![prop::collection::vec(inner.clone(), 0..5).prop_map(Value::from), prop::collection::hash_map(field(), inner, 0..6).prop_map(|m| Value::Object(m.into_iter().collect::<Map<_, _>>())),]
    })
}

/// The field names the inbox reads, so generated objects hit the code that reads them.
fn field() -> impl Strategy<Value = String> {
    prop::sample::select(vec![
        "repo", "number", "title", "url", "author", "authorName", "updatedAt", "isDraft", "isDirect", "review", "ci", "branch", "base", "failedChecks", "id", "status",
        "statusType", "parentId", "project", "expiresAt", "health", "app", "activated", "isRunning", "outcome", "failed", "job", "tests", "step", "workflowId", "dockerTag",
        "done", "total", "createdAt", "isLive", "tag", "dashboard", "workflow",
    ])
    .prop_map(str::to_string)
}

/// An inbox cache of this version whose every section holds anything.
fn inbox() -> impl Strategy<Value = Value> {
    let keys = ["direct", "team", "mine", "tickets", "errors", "merged", "deploys", "deployLinks", "previews", "botRepos"];
    prop::collection::vec(any_json(), keys.len()).prop_map(move |values| {
        let mut data = json!({"version": 1, "at": 1793181600000_i64});
        for (key, value) in keys.iter().zip(values) {
            data[*key] = value;
        }
        data
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]

    #[test]
    fn the_model_never_panics(data in prop_oneof![inbox(), any_json()], ci in any_json(), brief in any_json(), compact in any::<bool>(), width in 8u16..140) {
        let inputs = Inputs { data: Some(data), ci, brief: Some(brief), muted: vec!["api".into()], costs: HashMap::new() };
        let mut app = App::fixture(inputs, json!({}), compact, Local::fixed(HashMap::new(), Vec::new()), HashMap::new());
        render(&mut app, width, 30);
    }
}

//! The four tools and the drawing guidance as the plugin declares them in `plugin/hooks/tools.ts`, read at build time so
//! an agent on `cockpit mcp` reads the same words a plugin session does.

use std::sync::LazyLock;

use serde_json::{Value, json};

const SHARED_TS: &str = include_str!("../../../../../plugin/hooks/tools.ts");

/// The tools in the order `tools/list` gives them.
pub const NAMES: [&str; 4] = ["show", "cockpit_search", "cockpit_changes", "report_step"];
pub const PLACEMENTS: [&str; 4] = ["right", "down", "zoomed", "browser"];
pub const BLOCKS: [&str; 15] =
    ["markdown", "stats", "table", "chart", "flow", "sequence", "mermaid", "blueprint", "image", "compare", "diff", "log", "ci", "choice", "confirm"];
pub const OUTCOMES: [&str; 4] = ["done", "failed", "blocked", "waiting"];
pub const APPS: [&str; 2] = ["inbox", "monitor"];

/// The JSON a TypeScript source assigns to `export const <name>`, up to the last `close` bracket in the file.
pub fn json_in(source: &str, name: &str, close: char) -> Value {
    let at = source.find(&format!("export const {name}")).expect("the shared source names its constant");
    let start = at + source[at..].find(" = ").expect("the constant is assigned") + 3;
    let end = source.rfind(close).expect("the constant closes");
    serde_json::from_str(&source[start..=end]).expect("the shared source holds strict JSON")
}

static SHARED: LazyLock<Value> = LazyLock::new(|| json_in(SHARED_TS, "MCP", '}'));

fn strings(lines: &Value) -> Vec<&str> {
    lines.as_array().into_iter().flatten().filter_map(Value::as_str).collect()
}

/// The drawing guidance the server gives as its instructions.
pub fn instructions() -> String {
    strings(&SHARED["drawing"]).join("\n").replace("{show}", "cockpit's show tool")
}

/// What `tools/list` answers; the two that only read say so, so an agent may run them without asking.
pub fn list() -> Value {
    let tools = NAMES.iter().map(|name| {
        let tool = &SHARED["tools"][name];
        let mut out = json!({"name": tool["name"], "description": strings(&tool["description"]).concat(), "inputSchema": tool["inputSchema"]});
        if let Some(annotations) = tool.get("annotations") {
            out["annotations"] = annotations.clone();
        }
        out
    });
    Value::Array(tools.collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptions_are_the_plugins() {
        let tools = list();
        let said = |n: usize| tools[n]["description"].as_str().unwrap().to_string();
        assert!(said(0).starts_with("Draws charts, figures, tables") && said(0).ends_with("with rows given in the call."), "{}", said(0));
        // An escaped quote and a backslash read as themselves.
        assert!(said(0).contains("the person's next message") && said(0).contains(r"`---\ntitle:"));
        assert!(said(1).starts_with("Searches everything cockpit knows") && said(1).ends_with("a ticket's its id."));
        assert!(said(2).starts_with("What changed in the person's work") && said(2).ends_with("title}]}."));
        assert!(said(3).starts_with("Records how a runbook step") && said(3).ends_with("before it runs."));
        assert_eq!((&tools[1]["annotations"]["readOnlyHint"], &tools[0]["annotations"]), (&json!(true), &Value::Null));
        assert!(instructions().starts_with("# Drawing results\n") && !instructions().contains("{show}"));
    }

    #[test]
    fn the_schemas_name_what_the_checks_here_name() {
        assert_eq!(SHARED["tools"].as_object().unwrap().len(), NAMES.len());
        let props = |tool: &str| &SHARED["tools"][tool]["inputSchema"]["properties"];
        assert_eq!(strings(&props("show")["placement"]["enum"]), PLACEMENTS);
        assert_eq!(strings(&props("show")["blocks"]["items"]["properties"]["type"]["enum"]), BLOCKS);
        assert_eq!(strings(&props("cockpit_changes")["app"]["enum"]), APPS);
        assert_eq!(strings(&props("report_step")["outcome"]["enum"]), OUTCOMES);
        for name in NAMES {
            assert_eq!(SHARED["tools"][name]["name"], name);
            let schema = &SHARED["tools"][name]["inputSchema"];
            for required in strings(&schema["required"]) {
                assert!(schema["properties"].get(required).is_some(), "{name} requires {required}, which it does not declare");
            }
        }
    }
}

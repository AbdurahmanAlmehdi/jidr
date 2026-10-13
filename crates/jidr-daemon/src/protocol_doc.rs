//! Renders the command reference in docs/control-protocol.md from
//! `commands.list`, so the document can't drift from the code.

use jidr_core::provider::CommandSpec;
use serde_json::Value;

pub const BEGIN: &str = "<!-- commands:begin (generated from commands.list; JIDR_UPDATE_DOCS=1 cargo nextest run -p jidr-daemon protocol_doc) -->";
pub const END: &str = "<!-- commands:end -->";

fn type_of(schema: &Value, prop: &Value) -> String {
    if let Some(t) = prop.get("type") {
        let base = match t {
            Value::String(s) => s.clone(),
            Value::Array(ts) => ts
                .iter()
                .filter_map(Value::as_str)
                .filter(|t| *t != "null")
                .collect::<Vec<_>>()
                .join(" | "),
            _ => "any".into(),
        };
        if base == "array" {
            let item = prop
                .get("items")
                .map(|i| type_of(schema, i))
                .unwrap_or_else(|| "any".into());
            return format!("{item}[]");
        }
        if let Some(format) = prop.get("format").and_then(Value::as_str)
            && format == "uuid"
        {
            return "uuid".into();
        }
        return base;
    }
    let reference = prop
        .get("$ref")
        .or_else(|| {
            prop.get("anyOf")
                .and_then(|a| a.as_array())
                .and_then(|a| a.iter().find_map(|v| v.get("$ref")))
        })
        .and_then(Value::as_str);
    if let Some(r) = reference {
        let name = r.rsplit('/').next().unwrap_or(r);
        if let Some(variants) = schema
            .pointer(&format!("/$defs/{name}/enum"))
            .and_then(Value::as_array)
        {
            return variants
                .iter()
                .filter_map(Value::as_str)
                .map(|v| format!("`{v}`"))
                .collect::<Vec<_>>()
                .join(" \\| ");
        }
        return name.to_string();
    }
    "any".into()
}

fn description(schema: &Value, prop: &Value) -> String {
    let own = prop.get("description").and_then(Value::as_str);
    let via_ref = prop
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|r| {
            schema.pointer(&format!(
                "/$defs/{}/description",
                r.rsplit('/').next().unwrap_or(r)
            ))
        })
        .and_then(Value::as_str);
    own.or(via_ref)
        .unwrap_or("")
        .replace('\n', " ")
        .replace('|', "\\|")
}

pub fn render(specs: &[CommandSpec]) -> String {
    let mut out = String::new();
    let mut namespace = "";
    for s in specs {
        let ns = s.id.split('.').next().unwrap_or("");
        if ns != namespace {
            namespace = ns;
            out.push_str(&format!("\n### `{ns}.*`\n"));
        }
        let mut flags = Vec::new();
        if s.long_running {
            flags.push("long-running (job)");
        }
        if s.locks_device {
            flags.push("locks the device");
        }
        if s.destructive {
            flags.push("**destructive**");
        }
        out.push_str(&format!("\n#### `{}`\n\n{}", s.id, s.label));
        if !flags.is_empty() {
            out.push_str(&format!(" · {}", flags.join(" · ")));
        }
        out.push('\n');
        let required: Vec<&str> = s
            .params
            .get("required")
            .and_then(Value::as_array)
            .map(|r| r.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let props = s.params.get("properties").and_then(Value::as_object);
        match props {
            Some(props) if !props.is_empty() => {
                out.push_str("\n| Param | Type | Required | Description |\n|---|---|---|---|\n");
                for (name, prop) in props {
                    out.push_str(&format!(
                        "| `{name}` | {} | {} | {} |\n",
                        type_of(&s.params, prop),
                        if required.contains(&name.as_str()) {
                            "yes"
                        } else {
                            ""
                        },
                        description(&s.params, prop)
                    ));
                }
            }
            _ => out.push_str("\nNo params.\n"),
        }
    }
    out
}

/// Replaces the generated section of `doc` with `generated`.
pub fn splice(doc: &str, generated: &str) -> Option<String> {
    let start = doc.find(BEGIN)? + BEGIN.len();
    let end = doc.find(END)?;
    if end < start {
        return None;
    }
    Some(format!(
        "{}\n{}\n{}",
        &doc[..start],
        generated.trim_end(),
        &doc[end..]
    ))
}

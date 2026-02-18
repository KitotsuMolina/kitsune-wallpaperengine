use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize)]
pub struct ScriptAssignment {
    pub source_path: String,
    pub target_property: String,
    pub expression: String,
    pub depends_on_user: Option<String>,
    pub resolved_value: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScriptEvalResult {
    pub assignments: Vec<ScriptAssignment>,
    pub notes: Vec<String>,
}

fn try_parse_user_binding(v: &Value) -> Option<(String, Value)> {
    let obj = v.as_object()?;
    let value = obj.get("value")?.clone();

    if let Some(user_name) = obj.get("user").and_then(|u| u.as_str()) {
        return Some((user_name.to_string(), value));
    }

    if let Some(name) = obj
        .get("user")
        .and_then(|u| u.as_object())
        .and_then(|u| u.get("name"))
        .and_then(|u| u.as_str())
    {
        return Some((name.to_string(), value));
    }

    None
}

fn collect_user_bindings_recursive(v: &Value, out: &mut BTreeMap<String, Value>) {
    if let Some((k, vv)) = try_parse_user_binding(v)
        && !out.contains_key(&k)
    {
        out.insert(k, vv);
    }

    match v {
        Value::Object(map) => {
            for child in map.values() {
                collect_user_bindings_recursive(child, out);
            }
        }
        Value::Array(arr) => {
            for child in arr {
                collect_user_bindings_recursive(child, out);
            }
        }
        _ => {}
    }
}

fn collect_project_defaults(project_json: &Value, out: &mut BTreeMap<String, Value>) {
    let Some(props) = project_json
        .get("general")
        .and_then(|v| v.get("properties"))
        .and_then(|v| v.as_object())
    else {
        return;
    };

    for (name, prop) in props {
        let Some(default) = prop.get("value") else {
            continue;
        };
        out.entry(name.to_string()).or_insert_with(|| default.clone());
    }
}

fn parse_identifier(input: &str) -> Option<String> {
    let ident: String = input
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    if ident.is_empty() { None } else { Some(ident) }
}

fn parse_number(token: &str) -> Option<f64> {
    token.trim().parse::<f64>().ok()
}

fn value_to_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse::<f64>().ok(),
        _ => None,
    }
}

fn value_from_f64(v: f64) -> Value {
    serde_json::Number::from_f64(v)
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

fn eval_expression(expr: &str, user_values: &BTreeMap<String, Value>) -> Option<(Option<String>, Value)> {
    let clean = expr.trim();
    if clean.is_empty() {
        return None;
    }

    let (depends_on_user, mut current_value, mut rest) = if let Some(idx) = clean.find("changedUserProperties.") {
        let after = &clean[(idx + "changedUserProperties.".len())..];
        let user_key = parse_identifier(after)?;
        let base = user_values.get(&user_key)?.clone();
        let consumed = idx + "changedUserProperties.".len() + user_key.len();
        (Some(user_key), base, clean[consumed..].trim())
    } else if let Some(num) = parse_number(clean) {
        (None, value_from_f64(num), "")
    } else if clean.eq_ignore_ascii_case("true") || clean.eq_ignore_ascii_case("false") {
        (None, Value::Bool(clean.eq_ignore_ascii_case("true")), "")
    } else {
        return None;
    };

    while !rest.is_empty() {
        let op = rest.chars().next()?;
        if !matches!(op, '+' | '-' | '*' | '/') {
            break;
        }
        rest = rest[1..].trim_start();

        let token_len = rest
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
            .count();
        if token_len == 0 {
            break;
        }

        let token = &rest[..token_len];
        let rhs = parse_number(token)?;
        let lhs = value_to_f64(&current_value)?;
        let next = match op {
            '+' => lhs + rhs,
            '-' => lhs - rhs,
            '*' => lhs * rhs,
            '/' => {
                if rhs == 0.0 {
                    return None;
                }
                lhs / rhs
            }
            _ => return None,
        };
        current_value = value_from_f64(next);
        rest = rest[token_len..].trim_start();
    }

    Some((depends_on_user, current_value))
}

fn parse_script_assignments(script: &str, source_path: &str, user_values: &BTreeMap<String, Value>) -> Vec<ScriptAssignment> {
    let mut out = Vec::<ScriptAssignment>::new();

    for stmt in script.split(';') {
        let trimmed = stmt.trim();
        if !trimmed.contains("thisObject.") || !trimmed.contains('=') {
            continue;
        }

        let Some(eq_idx) = trimmed.rfind('=') else {
            continue;
        };
        let lhs = trimmed[..eq_idx].trim();
        let rhs = trimmed[(eq_idx + 1)..].trim();
        let Some(pos) = lhs.rfind("thisObject.") else {
            continue;
        };

        let target_raw = lhs[(pos + "thisObject.".len())..].trim();
        let target_property = parse_identifier(target_raw).unwrap_or_default();
        if target_property.is_empty() {
            continue;
        }

        let (depends_on_user, resolved_value) = match eval_expression(rhs, user_values) {
            Some((dep, value)) => (dep, Some(value)),
            None => (None, None),
        };

        out.push(ScriptAssignment {
            source_path: source_path.to_string(),
            target_property,
            expression: rhs.to_string(),
            depends_on_user,
            resolved_value,
        });
    }

    out
}

fn collect_scripts_recursive(
    v: &Value,
    path: &str,
    user_values: &BTreeMap<String, Value>,
    out: &mut Vec<ScriptAssignment>,
) {
    if let Some(obj) = v.as_object() {
        if let Some(script) = obj.get("script").and_then(|s| s.as_str()) {
            out.extend(parse_script_assignments(script, path, user_values));
        }

        for (k, child) in obj {
            let next = if path.is_empty() {
                k.to_string()
            } else {
                format!("{path}.{k}")
            };
            collect_scripts_recursive(child, &next, user_values, out);
        }
        return;
    }

    if let Some(arr) = v.as_array() {
        for (i, child) in arr.iter().enumerate() {
            let next = format!("{path}[{i}]");
            collect_scripts_recursive(child, &next, user_values, out);
        }
    }
}

pub fn collect_scene_user_properties(
    scene_json: &Value,
    project_json: Option<&Value>,
) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::<String, Value>::new();
    if let Some(project) = project_json {
        collect_project_defaults(project, &mut out);
    }
    collect_user_bindings_recursive(scene_json, &mut out);
    out
}

pub fn apply_scene_scripts(
    scene_json: &Value,
    user_values: &BTreeMap<String, Value>,
) -> ScriptEvalResult {
    let mut assignments = Vec::<ScriptAssignment>::new();
    collect_scripts_recursive(scene_json, "", user_values, &mut assignments);

    let mut notes = Vec::<String>::new();
    if !assignments.is_empty() {
        let resolved = assignments.iter().filter(|a| a.resolved_value.is_some()).count();
        notes.push(format!(
            "Script runtime (minimal) evaluated {resolved}/{} assignments",
            assignments.len()
        ));
    }

    ScriptEvalResult { assignments, notes }
}

pub fn to_json_object(values: &BTreeMap<String, Value>) -> Value {
    let mut obj = Map::new();
    for (k, v) in values {
        obj.insert(k.clone(), v.clone());
    }
    Value::Object(obj)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluates_simple_script_expression() {
        let mut users = BTreeMap::<String, Value>::new();
        users.insert("glow".to_string(), Value::from(2.0));
        let script = "if (changedUserProperties.hasOwnProperty('glow')) { thisObject.bloomstrength = changedUserProperties.glow * 0.5; }";
        let got = parse_script_assignments(script, "general.bloomstrength", &users);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].target_property, "bloomstrength");
        assert!(got[0].resolved_value.is_some());
    }
}

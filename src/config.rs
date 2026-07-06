use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use regex::Regex;
use serde_json::Value;

const JSON_DEPTH_LIMIT: usize = 32;
const SLACK_CHANNEL_ID_PATTERN: &str = r"^[CGDW][A-Z0-9]{8,}$";

pub const RULES_FILE_NAME: &str = "rules.json";

#[derive(Debug, Clone)]
pub struct Rule {
    pub index: usize,
    pub channel: String,
    pub matcher: Matcher,
    pub command: String,
}

#[derive(Debug, Clone)]
pub enum Matcher {
    Substring(String),
    Regex(Regex),
}

pub fn load_rules(path: &Path) -> Result<Vec<Rule>> {
    let bytes =
        fs::read(path).with_context(|| format!("failed to read config file {}", path.display()))?;

    let value: Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse config file {}", path.display()))?;

    let max = max_depth(&value, 0);
    if max > JSON_DEPTH_LIMIT {
        bail!(
            "config file exceeds JSON depth limit ({} > {})",
            max,
            JSON_DEPTH_LIMIT
        );
    }

    let array = value
        .as_array()
        .with_context(|| "config file must be a JSON array of rule objects")?;

    let mut rules = Vec::with_capacity(array.len());
    for (index, item) in array.iter().enumerate() {
        let rule =
            parse_rule(index, item).with_context(|| format!("invalid rule at index {index}"))?;
        rules.push(rule);
    }

    Ok(rules)
}

fn parse_rule(index: usize, value: &Value) -> Result<Rule> {
    let object = value
        .as_object()
        .with_context(|| "rule must be a JSON object")?;

    let channel = object
        .get("channel")
        .and_then(Value::as_str)
        .with_context(|| "missing or non-string `channel`")?
        .trim()
        .to_owned();
    if channel.is_empty() {
        bail!("`channel` must not be empty");
    }

    let message = object
        .get("message")
        .and_then(Value::as_str)
        .with_context(|| "missing or non-string `message`")?
        .to_owned();
    if message.is_empty() {
        bail!("`message` must not be empty");
    }

    let command = object
        .get("command")
        .and_then(Value::as_str)
        .with_context(|| "missing or non-string `command`")?
        .to_owned();
    if command.is_empty() {
        bail!("`command` must not be empty");
    }

    let is_regex = object
        .get("regex")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let matcher = if is_regex {
        let regex = Regex::new(&message).with_context(|| format!("invalid regex `{}`", message))?;
        Matcher::Regex(regex)
    } else {
        Matcher::Substring(message)
    };

    Ok(Rule {
        index,
        channel,
        matcher,
        command,
    })
}

fn max_depth(value: &Value, current: usize) -> usize {
    let mut deepest = current;
    match value {
        Value::Object(map) => {
            for v in map.values() {
                deepest = deepest.max(max_depth(v, current + 1));
            }
        }
        Value::Array(arr) => {
            for v in arr {
                deepest = deepest.max(max_depth(v, current + 1));
            }
        }
        _ => {}
    }
    deepest
}

pub fn looks_like_channel_id(name: &str) -> bool {
    Regex::new(SLACK_CHANNEL_ID_PATTERN)
        .expect("constant regex compiles")
        .is_match(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp_config(body: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(body.as_bytes()).unwrap();
        f
    }

    #[test]
    fn loads_simple_substring_rule() {
        let f =
            write_temp_config(r#"[{"channel":"general","message":"ping","command":"echo pong"}]"#);
        let rules = load_rules(f.path()).unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].channel, "general");
        assert!(matches!(rules[0].matcher, Matcher::Substring(ref s) if s == "ping"));
        assert_eq!(rules[0].command, "echo pong");
    }

    #[test]
    fn compiles_regex_rule() {
        let f = write_temp_config(
            r#"[{"channel":"c","message":"^deploy (prod|staging)$","regex":true,"command":"echo deploy"}]"#,
        );
        let rules = load_rules(f.path()).unwrap();
        assert!(matches!(rules[0].matcher, Matcher::Regex(_)));
    }

    #[test]
    fn rejects_invalid_regex_with_index() {
        let f = write_temp_config(
            r#"[{"channel":"c","message":"[unterminated","regex":true,"command":"x"}]"#,
        );
        let err = load_rules(f.path()).unwrap_err().to_string();
        assert!(err.contains("invalid rule at index 0"), "got: {err}");
    }

    #[test]
    fn rejects_empty_channel() {
        let f = write_temp_config(r#"[{"channel":"","message":"x","command":"y"}]"#);
        assert!(load_rules(f.path()).is_err());
    }

    #[test]
    fn rejects_empty_command() {
        let f = write_temp_config(r#"[{"channel":"c","message":"x","command":""}]"#);
        assert!(load_rules(f.path()).is_err());
    }

    #[test]
    fn rejects_non_array_root() {
        let f = write_temp_config(r#"{"channel":"c"}"#);
        let err = load_rules(f.path()).unwrap_err().to_string();
        assert!(err.contains("must be a JSON array"));
    }

    #[test]
    fn rejects_oversized_depth() {
        let mut body = String::from("[");
        for _ in 0..40 {
            body.push_str("{\"a\":");
        }
        body.push('1');
        for _ in 0..40 {
            body.push('}');
        }
        body.push(']');
        let f = write_temp_config(&body);
        let err = load_rules(f.path()).unwrap_err().to_string();
        assert!(err.contains("depth limit"), "got: {err}");
    }

    #[test]
    fn channel_id_pattern_matches_known_shapes() {
        assert!(looks_like_channel_id("C0123ABCD"));
        assert!(looks_like_channel_id("G0123ABCD"));
        assert!(looks_like_channel_id("D0123ABCD"));
        assert!(looks_like_channel_id("W0123ABCD"));
        assert!(!looks_like_channel_id("general"));
        assert!(!looks_like_channel_id("C012"));
    }
}

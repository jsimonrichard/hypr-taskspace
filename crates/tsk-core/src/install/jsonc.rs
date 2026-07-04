use regex::Regex;
use serde_json::Value;

use crate::error::{TskError, Result};

pub fn parse_jsonc(text: &str) -> Result<Value> {
    let block = Regex::new(r"/\*.*?\*/").unwrap();
    let line = Regex::new(r"(?m)//[^\n]*").unwrap();
    let stripped = block.replace_all(text, "");
    let stripped = line.replace_all(&stripped, "");
    serde_json::from_str(stripped.trim()).map_err(|source| TskError::Parse {
        path: "config.jsonc".into(),
        source,
    })
}

pub fn dump_jsonc(value: &Value) -> String {
    format!("{}\n", serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_line_comments() {
        let raw = r#"{
  // comment
  "a": 1
}"#;
        let v = parse_jsonc(raw).unwrap();
        assert_eq!(v["a"], 1);
    }
}

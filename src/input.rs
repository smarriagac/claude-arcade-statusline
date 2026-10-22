//! The JSON document Claude Code pipes to the status line command on stdin.
//!
//! Every field is optional and lenient: a missing, `null` or wrongly typed
//! value becomes `None` instead of an error, so the HUD can hide that segment
//! silently.

use serde::Deserialize;
use serde::de::{DeserializeOwned, Deserializer};
use serde_json::Value;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct StatusInput {
    #[serde(deserialize_with = "lenient")]
    pub model: Option<Model>,
    #[serde(deserialize_with = "lenient")]
    pub workspace: Option<Workspace>,
    /// Older Claude Code builds send the directory here instead of `workspace`.
    #[serde(deserialize_with = "lenient_str")]
    pub cwd: Option<String>,
    #[serde(deserialize_with = "lenient")]
    pub cost: Option<Cost>,
    #[serde(deserialize_with = "lenient")]
    pub context_window: Option<ContextWindow>,
    #[serde(deserialize_with = "lenient")]
    pub rate_limits: Option<RateLimits>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Model {
    #[serde(deserialize_with = "lenient_str")]
    pub display_name: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Workspace {
    #[serde(deserialize_with = "lenient_str")]
    pub current_dir: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Cost {
    #[serde(deserialize_with = "lenient_num")]
    pub total_cost_usd: Option<f64>,
    #[serde(deserialize_with = "lenient_num")]
    pub total_duration_ms: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ContextWindow {
    #[serde(deserialize_with = "lenient_num")]
    pub used_percentage: Option<f64>,
    #[serde(deserialize_with = "lenient_num")]
    pub context_window_size: Option<f64>,
    #[serde(deserialize_with = "lenient_num")]
    pub total_input_tokens: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct RateLimits {
    #[serde(deserialize_with = "lenient")]
    pub five_hour: Option<Limit>,
    #[serde(deserialize_with = "lenient")]
    pub seven_day: Option<Limit>,
    #[serde(deserialize_with = "lenient")]
    pub spend_limit: Option<Limit>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Limit {
    #[serde(deserialize_with = "lenient_num")]
    pub used_percentage: Option<f64>,
    /// Epoch seconds.
    #[serde(deserialize_with = "lenient_num")]
    pub resets_at: Option<f64>,
}

impl StatusInput {
    /// Parses stdin. Invalid JSON yields an empty input (every segment hidden).
    pub fn parse(text: &str) -> Self {
        serde_json::from_str(text).unwrap_or_default()
    }

    pub fn model_name(&self) -> Option<&str> {
        self.model.as_ref()?.display_name.as_deref()
    }

    /// `workspace.current_dir`, falling back to the legacy `cwd` field.
    pub fn current_dir(&self) -> Option<&str> {
        self.workspace
            .as_ref()
            .and_then(|w| w.current_dir.as_deref())
            .or(self.cwd.as_deref())
    }
}

/// Accepts any JSON value; anything that does not fit `T` becomes `None`.
fn lenient<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let value = Value::deserialize(deserializer)?;
    Ok(T::deserialize(value).ok())
}

/// Non-empty strings only (jq's `// ""` treats empty strings as missing).
fn lenient_str<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(match Value::deserialize(deserializer)? {
        Value::String(s) if !s.is_empty() => Some(s),
        _ => None,
    })
}

/// Finite numbers, also when sent as numeric strings.
fn lenient_num<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: Deserializer<'de>,
{
    let number = match Value::deserialize(deserializer)? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    };
    Ok(number.filter(|n| n.is_finite()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_document() {
        let input = StatusInput::parse(
            r#"{"model":{"display_name":"Opus"},"workspace":{"current_dir":"/a/b"},
                "cost":{"total_cost_usd":1.23,"total_duration_ms":125000},
                "context_window":{"used_percentage":42,"context_window_size":200000,"total_input_tokens":84000},
                "rate_limits":{"five_hour":{"used_percentage":63,"resets_at":1700000000},
                               "seven_day":{"used_percentage":85.5}}}"#,
        );
        assert_eq!(input.model_name(), Some("Opus"));
        assert_eq!(input.current_dir(), Some("/a/b"));
        let limits = input.rate_limits.as_ref().unwrap();
        assert_eq!(limits.five_hour.as_ref().unwrap().resets_at, Some(1.7e9));
        assert_eq!(
            limits.seven_day.as_ref().unwrap().used_percentage,
            Some(85.5)
        );
        assert!(limits.spend_limit.is_none());
    }

    #[test]
    fn invalid_json_is_empty_input() {
        let input = StatusInput::parse("not json at all");
        assert!(input.model_name().is_none());
        assert!(input.current_dir().is_none());
        assert!(input.cost.is_none());
    }

    #[test]
    fn nulls_wrong_types_and_empty_strings_become_none() {
        let input = StatusInput::parse(
            r#"{"model":"Opus","workspace":{"current_dir":""},"cwd":"/legacy",
                "cost":{"total_cost_usd":null,"total_duration_ms":"9000"},
                "context_window":{"used_percentage":true},
                "rate_limits":null}"#,
        );
        assert!(input.model.is_none());
        assert_eq!(input.current_dir(), Some("/legacy"));
        let cost = input.cost.as_ref().unwrap();
        assert_eq!(cost.total_cost_usd, None);
        assert_eq!(cost.total_duration_ms, Some(9000.0));
        assert_eq!(input.context_window.as_ref().unwrap().used_percentage, None);
        assert!(input.rate_limits.is_none());
    }
}

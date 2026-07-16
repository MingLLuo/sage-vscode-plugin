//! Validated analysis-budget profiles supplied by the editor client.
//!
//! The mode intentionally controls bounded, workspace-wide result breadth rather
//! than changing symbol-resolution semantics. That keeps navigation answers
//! consistent across profiles while still giving the public setting a measurable
//! performance/coverage trade-off.

use serde::{Deserialize, Deserializer};
use serde_json::Value;

const LIGHT_WORKSPACE_SYMBOL_LIMIT: usize = 50;
const DEFAULT_WORKSPACE_SYMBOL_LIMIT: usize = 200;
const FULL_WORKSPACE_SYMBOL_LIMIT: usize = 1_000;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum AnalysisMode {
    Light,
    #[default]
    Default,
    Full,
}

impl AnalysisMode {
    pub(super) fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "light" => Some(Self::Light),
            "default" => Some(Self::Default),
            "full" => Some(Self::Full),
            _ => None,
        }
    }

    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Default => "default",
            Self::Full => "full",
        }
    }

    pub(super) const fn workspace_symbol_limit(self) -> usize {
        match self {
            Self::Light => LIGHT_WORKSPACE_SYMBOL_LIMIT,
            Self::Default => DEFAULT_WORKSPACE_SYMBOL_LIMIT,
            Self::Full => FULL_WORKSPACE_SYMBOL_LIMIT,
        }
    }
}

/// Keeps an invalid client value available for a user-facing warning while
/// safely applying the documented default profile. A malformed mode must not
/// discard unrelated initialization options.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct ConfiguredAnalysisMode {
    effective: AnalysisMode,
    invalid_value: Option<String>,
}

impl ConfiguredAnalysisMode {
    pub(super) const fn effective(&self) -> AnalysisMode {
        self.effective
    }

    pub(super) fn invalid_value(&self) -> Option<&str> {
        self.invalid_value.as_deref()
    }
}

impl<'de> Deserialize<'de> for ConfiguredAnalysisMode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        let raw = value.as_str();
        Ok(match raw.and_then(AnalysisMode::parse) {
            Some(effective) => Self {
                effective,
                invalid_value: None,
            },
            None => Self {
                effective: AnalysisMode::default(),
                invalid_value: Some(raw.map(str::to_string).unwrap_or_else(|| value.to_string())),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_modes_have_distinct_workspace_symbol_budgets() {
        assert_eq!(AnalysisMode::parse("light"), Some(AnalysisMode::Light));
        assert_eq!(
            AnalysisMode::parse(" DEFAULT "),
            Some(AnalysisMode::Default)
        );
        assert_eq!(AnalysisMode::parse("FULL"), Some(AnalysisMode::Full));
        assert_eq!(AnalysisMode::Light.workspace_symbol_limit(), 50);
        assert_eq!(AnalysisMode::Default.workspace_symbol_limit(), 200);
        assert_eq!(AnalysisMode::Full.workspace_symbol_limit(), 1_000);
    }

    #[test]
    fn invalid_mode_falls_back_without_hiding_the_bad_value() {
        let configured: ConfiguredAnalysisMode = serde_json::from_str("\"maximum\"").unwrap();
        assert_eq!(configured.effective(), AnalysisMode::Default);
        assert_eq!(configured.invalid_value(), Some("maximum"));

        let malformed: ConfiguredAnalysisMode = serde_json::from_str("42").unwrap();
        assert_eq!(malformed.effective(), AnalysisMode::Default);
        assert_eq!(malformed.invalid_value(), Some("42"));
    }
}

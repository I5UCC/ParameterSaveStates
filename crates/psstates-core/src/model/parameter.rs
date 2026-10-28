//! Typed avatar parameter values.

use std::fmt;

use serde::{Deserialize, Serialize};

/// OSC parameter types supported by VRChat avatar parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParamType {
    Float,
    Int,
    Bool,
}

impl ParamType {
    pub const ALL: [ParamType; 3] = [ParamType::Float, ParamType::Int, ParamType::Bool];

    /// OSC / OSCQuery type tag.
    pub fn osc_tag(self) -> &'static str {
        match self {
            ParamType::Float => "f",
            ParamType::Int => "i",
            ParamType::Bool => "T",
        }
    }

    /// Parses an OSC / OSCQuery type tag (`f`, `i`, `T`, `F`).
    pub fn from_osc_tag(tag: &str) -> Option<Self> {
        match tag {
            "f" => Some(ParamType::Float),
            "i" => Some(ParamType::Int),
            "T" | "F" => Some(ParamType::Bool),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ParamType::Float => "Float",
            ParamType::Int => "Int",
            ParamType::Bool => "Bool",
        }
    }

    /// A sensible default value for this type.
    pub fn default_value(self) -> ParamValue {
        match self {
            ParamType::Float => ParamValue::Float(0.0),
            ParamType::Int => ParamValue::Int(0),
            ParamType::Bool => ParamValue::Bool(false),
        }
    }
}

impl fmt::Display for ParamType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// A typed avatar parameter value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "lowercase")]
pub enum ParamValue {
    Float(f32),
    Int(i32),
    Bool(bool),
}

impl ParamValue {
    pub fn param_type(&self) -> ParamType {
        match self {
            ParamValue::Float(_) => ParamType::Float,
            ParamValue::Int(_) => ParamType::Int,
            ParamValue::Bool(_) => ParamType::Bool,
        }
    }

    /// Parses a textual value of the given type. Always culture invariant.
    pub fn parse(ty: ParamType, text: &str) -> std::result::Result<Self, String> {
        let text = text.trim();
        match ty {
            ParamType::Float => {
                let v: f32 = text
                    .parse()
                    .map_err(|_| format!("'{text}' is not a valid float"))?;
                if !v.is_finite() {
                    return Err(format!("'{text}' is not a finite float"));
                }
                Ok(ParamValue::Float(v))
            }
            ParamType::Int => text
                .parse::<i32>()
                .map(ParamValue::Int)
                .map_err(|_| format!("'{text}' is not a valid int")),
            ParamType::Bool => {
                if text.eq_ignore_ascii_case("true") {
                    Ok(ParamValue::Bool(true))
                } else if text.eq_ignore_ascii_case("false") {
                    Ok(ParamValue::Bool(false))
                } else {
                    Err(format!("'{text}' is not a valid bool (true/false)"))
                }
            }
        }
    }

    /// Canonical text representation (round-trips through [`ParamValue::parse`]).
    pub fn to_text(&self) -> String {
        match self {
            ParamValue::Float(v) => v.to_string(),
            ParamValue::Int(v) => v.to_string(),
            ParamValue::Bool(v) => if *v { "true" } else { "false" }.to_string(),
        }
    }
}

impl fmt::Display for ParamValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_text())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_values_invariant() {
        assert_eq!(
            ParamValue::parse(ParamType::Float, " 0.25 ").unwrap(),
            ParamValue::Float(0.25)
        );
        assert!(ParamValue::parse(ParamType::Float, "0,25").is_err());
        assert!(ParamValue::parse(ParamType::Float, "NaN").is_err());
        assert_eq!(
            ParamValue::parse(ParamType::Int, "-3").unwrap(),
            ParamValue::Int(-3)
        );
        assert_eq!(
            ParamValue::parse(ParamType::Bool, "True").unwrap(),
            ParamValue::Bool(true)
        );
        assert!(ParamValue::parse(ParamType::Bool, "1").is_err());
        assert_eq!(ParamValue::Float(1.0).to_text(), "1");
    }

    #[test]
    fn value_json_roundtrip() {
        let json = serde_json::to_string(&ParamValue::Int(3)).unwrap();
        assert_eq!(json, r#"{"type":"int","value":3}"#);
        let back: ParamValue = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ParamValue::Int(3));
    }
}

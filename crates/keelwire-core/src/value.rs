//! Dynamic decoded field value shared by the CANboat decoder and the
//! proprietary schema codec.

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(untagged))]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    Bytes(Vec<u8>),
    List(Vec<Vec<(String, Value)>>),
}

impl Value {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Int(i) => Some(*i as f64),
            Value::Float(f) => Some(*f),
            Value::Bool(b) => Some(*b as u8 as f64),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            Value::Float(f) => Some(f.round() as i64),
            Value::Bool(b) => Some(*b as i64),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Text(s) => Some(s),
            _ => None,
        }
    }

    pub fn display(&self) -> String {
        match self {
            Value::Null => "-".into(),
            Value::Bool(b) => b.to_string(),
            Value::Int(i) => i.to_string(),
            Value::Float(f) => {
                if f.abs() >= 1000.0 || f.fract() == 0.0 {
                    format!("{:.1}", f)
                } else {
                    let s = format!("{:.5}", f);
                    s.trim_end_matches('0').trim_end_matches('.').to_string()
                }
            }
            Value::Text(s) => s.clone(),
            Value::Bytes(b) => crate::frame::hex(b),
            Value::List(l) => format!("[{} items]", l.len()),
        }
    }
}

/// A decoded field: id, display name, value, optional unit and lookup text.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Field {
    pub id: String,
    pub name: String,
    pub value: Value,
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none", default))]
    pub unit: Option<String>,
    /// Text for LOOKUP-type fields (value holds the raw number).
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none", default))]
    pub text: Option<String>,
}

impl Field {
    /// Human-friendly rendering: SI units from the PGN database converted to
    /// the display units chosen in Settings (see `units`).
    pub fn friendly(&self) -> String {
        if self.text.is_none() && !matches!(self.value, Value::Null) {
            if let (Some(v), Some(u)) = (self.value.as_f64(), self.unit.as_deref()) {
                if let Some(s) = crate::units::format(v, u, &self.id, &crate::units::current()) {
                    return s;
                }
            }
        }
        self.render()
    }

    pub fn render(&self) -> String {
        if let Some(t) = &self.text {
            return t.clone();
        }
        match &self.unit {
            Some(u) if !matches!(self.value, Value::Null) => format!("{} {}", self.value.display(), u),
            _ => self.value.display(),
        }
    }
}

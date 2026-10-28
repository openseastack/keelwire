//! Data-driven bit-packed codec for proprietary payloads.
//!
//! A schema is a list of sequential fields. Layouts recovered by protocol
//! extraction are stored as JSON files and loaded at runtime, so new device
//! layouts need no code changes. Schemas carry a `verified` flag; the engine
//! refuses to *write* with unverified schemas unless explicitly overridden.

use crate::bits::{get_bits, set_bits, sign_extend};
use crate::iso::{encode_string_lau, string_lau};
use crate::value::Value;

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(rename_all = "snake_case"))]
pub enum FieldKind {
    Uint,
    Int,
    Bool,
    Enum,
    StringFix,
    StringLau,
    Bytes,
    Reserved,
    Group,
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(untagged))]
pub enum Count {
    Fixed(usize),
    Field(String),
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EnumValue {
    pub value: i64,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SchemaField {
    pub name: String,
    #[cfg_attr(feature = "serde", serde(default))]
    pub label: Option<String>,
    pub kind: FieldKind,
    /// Bit width (for strings: total byte length × 8 for string_fix; ignored for string_lau / group).
    #[cfg_attr(feature = "serde", serde(default))]
    pub bits: usize,
    #[cfg_attr(feature = "serde", serde(default = "one"))]
    pub scale: f64,
    #[cfg_attr(feature = "serde", serde(default))]
    pub offset: f64,
    #[cfg_attr(feature = "serde", serde(default))]
    pub unit: Option<String>,
    #[cfg_attr(feature = "serde", serde(default))]
    pub readonly: bool,
    #[cfg_attr(feature = "serde", serde(default))]
    pub min: Option<f64>,
    #[cfg_attr(feature = "serde", serde(default))]
    pub max: Option<f64>,
    #[cfg_attr(feature = "serde", serde(default))]
    pub values: Vec<EnumValue>,
    /// For groups: repetition count.
    #[cfg_attr(feature = "serde", serde(default))]
    pub count: Option<Count>,
    #[cfg_attr(feature = "serde", serde(default))]
    pub fields: Vec<SchemaField>,
}

#[allow(dead_code)]
fn one() -> f64 {
    1.0
}

impl SchemaField {
    pub fn uint(name: &str, bits: usize) -> Self {
        SchemaField {
            name: name.into(),
            label: None,
            kind: FieldKind::Uint,
            bits,
            scale: 1.0,
            offset: 0.0,
            unit: None,
            readonly: false,
            min: None,
            max: None,
            values: vec![],
            count: None,
            fields: vec![],
        }
    }
    pub fn of(kind: FieldKind, name: &str, bits: usize) -> Self {
        SchemaField { kind, ..Self::uint(name, bits) }
    }
    pub fn scaled(mut self, scale: f64, unit: &str) -> Self {
        self.scale = scale;
        self.unit = Some(unit.into());
        self
    }
    pub fn group(name: &str, count: Count, fields: Vec<SchemaField>) -> Self {
        SchemaField { count: Some(count), fields, ..Self::of(FieldKind::Group, name, 0) }
    }
}

/// How to ask a device for (or send it) the payload a schema describes:
/// the Maretron command byte plus any fixed data bytes before the payload.
#[derive(Clone, Debug, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CommandSpec {
    pub command: u8,
    /// Hex bytes sent after the command byte (e.g. a sub-command), before any payload.
    #[cfg_attr(feature = "serde", serde(default))]
    pub prefix: String,
}

impl CommandSpec {
    pub fn prefix_bytes(&self) -> Vec<u8> {
        crate::frame::parse_hex(&self.prefix).unwrap_or_default()
    }
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Schema {
    pub id: String,
    #[cfg_attr(feature = "serde", serde(default))]
    pub description: String,
    pub manufacturer: u16,
    #[cfg_attr(feature = "serde", serde(default = "marine"))]
    pub industry: u8,
    pub pgn: u32,
    #[cfg_attr(feature = "serde", serde(default))]
    pub product_code: Option<u16>,
    #[cfg_attr(feature = "serde", serde(default))]
    pub model: Option<String>,
    /// Response command byte(s) whose payload this schema describes.
    #[cfg_attr(feature = "serde", serde(default))]
    pub opcodes: Vec<u8>,
    /// Request that makes the device reply with this payload.
    #[cfg_attr(feature = "serde", serde(default))]
    pub read: Option<CommandSpec>,
    /// Request that writes this payload (data = prefix + encoded payload).
    #[cfg_attr(feature = "serde", serde(default))]
    pub write: Option<CommandSpec>,
    #[cfg_attr(feature = "serde", serde(default))]
    pub verified: bool,
    #[cfg_attr(feature = "serde", serde(default))]
    pub source: String,
    pub fields: Vec<SchemaField>,
}

#[allow(dead_code)]
fn marine() -> u8 {
    4
}

pub type Record = Vec<(String, Value)>;

/// JSON object → Record (nested arrays of objects become `Value::List`).
#[cfg(feature = "serde")]
pub fn record_from_json(v: &serde_json::Value) -> Record {
    fn conv(v: &serde_json::Value) -> Value {
        match v {
            serde_json::Value::Null => Value::Null,
            serde_json::Value::Bool(b) => Value::Bool(*b),
            serde_json::Value::Number(n) => n.as_i64().map(Value::Int).unwrap_or_else(|| Value::Float(n.as_f64().unwrap_or(0.0))),
            serde_json::Value::String(s) => Value::Text(s.clone()),
            serde_json::Value::Array(a) => Value::List(a.iter().map(record_from_json).collect()),
            serde_json::Value::Object(_) => Value::List(vec![record_from_json(v)]),
        }
    }
    match v.as_object() {
        Some(o) => o.iter().map(|(k, v)| (k.clone(), conv(v))).collect(),
        None => vec![],
    }
}

/// Record → JSON object.
#[cfg(feature = "serde")]
pub fn record_to_json(r: &Record) -> serde_json::Value {
    fn conv(v: &Value) -> serde_json::Value {
        match v {
            Value::Null => serde_json::Value::Null,
            Value::Bool(b) => (*b).into(),
            Value::Int(i) => (*i).into(),
            Value::Float(f) => serde_json::json!(f),
            Value::Text(s) => s.clone().into(),
            Value::Bytes(b) => crate::frame::hex(b).into(),
            Value::List(l) => serde_json::Value::Array(l.iter().map(record_to_json).collect()),
        }
    }
    serde_json::Value::Object(r.iter().map(|(k, v)| (k.clone(), conv(v))).collect())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodecError {
    Truncated(String),
    Missing(String),
    Range(String),
    Type(String),
}

impl std::fmt::Display for CodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CodecError::Truncated(n) => write!(f, "payload truncated at field '{n}'"),
            CodecError::Missing(n) => write!(f, "missing value for field '{n}'"),
            CodecError::Range(n) => write!(f, "value out of range for field '{n}'"),
            CodecError::Type(n) => write!(f, "wrong value type for field '{n}'"),
        }
    }
}

impl std::error::Error for CodecError {}

fn lookup<'a>(rec: &'a Record, name: &str) -> Option<&'a Value> {
    rec.iter().find(|(k, _)| k == name).map(|(_, v)| v)
}

fn decode_fields(fields: &[SchemaField], data: &[u8], bit: &mut usize, out: &mut Record) -> Result<(), CodecError> {
    for f in fields {
        let v = match f.kind {
            FieldKind::Group => {
                let n = match &f.count {
                    Some(Count::Fixed(n)) => *n,
                    Some(Count::Field(name)) => lookup(out, name).and_then(|v| v.as_i64()).ok_or_else(|| CodecError::Missing(name.clone()))? as usize,
                    None => 1,
                };
                let mut items = Vec::with_capacity(n);
                for _ in 0..n {
                    let mut rec = Vec::new();
                    decode_fields(&f.fields, data, bit, &mut rec)?;
                    items.push(rec);
                }
                Value::List(items)
            }
            FieldKind::StringLau => {
                let start = (*bit).div_ceil(8);
                let (s, n) = string_lau(data.get(start..).unwrap_or(&[])).ok_or_else(|| CodecError::Truncated(f.name.clone()))?;
                *bit = (start + n) * 8;
                Value::Text(s)
            }
            FieldKind::StringFix | FieldKind::Bytes => {
                let start = *bit / 8;
                let len = f.bits / 8;
                let b = data.get(start..start + len).ok_or_else(|| CodecError::Truncated(f.name.clone()))?;
                *bit += f.bits;
                if f.kind == FieldKind::StringFix {
                    Value::Text(crate::iso::string_fix(b))
                } else {
                    Value::Bytes(b.to_vec())
                }
            }
            _ => {
                let raw = get_bits(data, *bit, f.bits).ok_or_else(|| CodecError::Truncated(f.name.clone()))?;
                *bit += f.bits;
                match f.kind {
                    FieldKind::Reserved => continue,
                    FieldKind::Bool => Value::Bool(raw != 0),
                    FieldKind::Enum => Value::Int(raw as i64),
                    FieldKind::Int => {
                        let i = sign_extend(raw, f.bits);
                        scaled(i as f64, f)
                    }
                    _ => scaled(raw as f64, f),
                }
            }
        };
        out.push((f.name.clone(), v));
    }
    Ok(())
}

fn scaled(raw: f64, f: &SchemaField) -> Value {
    if f.scale == 1.0 && f.offset == 0.0 {
        Value::Int(raw as i64)
    } else {
        Value::Float(raw * f.scale + f.offset)
    }
}

fn encode_fields(fields: &[SchemaField], rec: &Record, out: &mut Vec<u8>, bit: &mut usize) -> Result<(), CodecError> {
    for f in fields {
        match f.kind {
            FieldKind::Reserved => {
                set_bits(out, *bit, f.bits, crate::bits::max_unsigned(f.bits));
                *bit += f.bits;
            }
            FieldKind::Group => {
                let items = match lookup(rec, &f.name) {
                    Some(Value::List(l)) => l.clone(),
                    Some(_) => return Err(CodecError::Type(f.name.clone())),
                    None => vec![],
                };
                let n = match &f.count {
                    Some(Count::Fixed(n)) => *n,
                    _ => items.len(),
                };
                for i in 0..n {
                    let empty = Vec::new();
                    let item = items.get(i).unwrap_or(&empty);
                    encode_fields(&f.fields, item, out, bit)?;
                }
            }
            FieldKind::StringLau => {
                let s = lookup(rec, &f.name).and_then(|v| v.as_str()).unwrap_or("");
                let start = (*bit).div_ceil(8);
                out.resize(start, 0xFF);
                out.extend(encode_string_lau(s));
                *bit = out.len() * 8;
            }
            FieldKind::StringFix | FieldKind::Bytes => {
                let len = f.bits / 8;
                let start = *bit / 8;
                let bytes = match lookup(rec, &f.name) {
                    Some(Value::Text(s)) => crate::iso::encode_string_fix(s, len),
                    Some(Value::Bytes(b)) => {
                        let mut b = b.clone();
                        b.resize(len, 0xFF);
                        b
                    }
                    None => vec![0xFF; len],
                    _ => return Err(CodecError::Type(f.name.clone())),
                };
                if out.len() < start + len {
                    out.resize(start + len, 0xFF);
                }
                out[start..start + len].copy_from_slice(&bytes);
                *bit += f.bits;
            }
            _ => {
                let raw: u64 = match lookup(rec, &f.name) {
                    None | Some(Value::Null) => crate::bits::max_unsigned(f.bits),
                    Some(v) => {
                        let x = v.as_f64().ok_or_else(|| CodecError::Type(f.name.clone()))?;
                        if let Some(min) = f.min {
                            if x < min - 1e-9 {
                                return Err(CodecError::Range(f.name.clone()));
                            }
                        }
                        if let Some(max) = f.max {
                            if x > max + 1e-9 {
                                return Err(CodecError::Range(f.name.clone()));
                            }
                        }
                        let r = ((x - f.offset) / f.scale).round();
                        if f.kind == FieldKind::Int {
                            let lim = 1i64 << (f.bits - 1);
                            if r < -(lim as f64) || r >= lim as f64 {
                                return Err(CodecError::Range(f.name.clone()));
                            }
                            (r as i64 as u64) & crate::bits::max_unsigned(f.bits)
                        } else {
                            if r < 0.0 || r > crate::bits::max_unsigned(f.bits) as f64 {
                                return Err(CodecError::Range(f.name.clone()));
                            }
                            r as u64
                        }
                    }
                };
                set_bits(out, *bit, f.bits, raw);
                *bit += f.bits;
            }
        }
    }
    Ok(())
}

impl Schema {
    pub fn decode(&self, payload: &[u8]) -> Result<Record, CodecError> {
        let mut out = Vec::new();
        let mut bit = 0;
        decode_fields(&self.fields, payload, &mut bit, &mut out)?;
        Ok(out)
    }

    pub fn encode(&self, rec: &Record) -> Result<Vec<u8>, CodecError> {
        let mut out = Vec::new();
        let mut bit = 0;
        encode_fields(&self.fields, rec, &mut out, &mut bit)?;
        out.resize(bit.div_ceil(8), 0xFF);
        Ok(out)
    }

    pub fn matches(&self, manufacturer: u16, pgn: u32, product_code: Option<u16>, opcode: Option<u8>) -> bool {
        self.manufacturer == manufacturer
            && self.pgn == pgn
            && (self.product_code.is_none() || self.product_code == product_code)
            && (self.opcodes.is_empty() || opcode.map(|o| self.opcodes.contains(&o)).unwrap_or(false))
    }
}

/// In-memory registry of schemas.
#[derive(Default, Clone)]
pub struct SchemaRegistry {
    pub schemas: Vec<Schema>,
}

impl SchemaRegistry {
    pub fn add(&mut self, s: Schema) {
        self.schemas.retain(|x| x.id != s.id);
        self.schemas.push(s);
    }

    pub fn find(&self, manufacturer: u16, pgn: u32, product_code: Option<u16>, opcode: Option<u8>) -> Option<&Schema> {
        // Prefer the most specific match (product code set).
        self.schemas
            .iter()
            .filter(|s| s.matches(manufacturer, pgn, product_code, opcode))
            .max_by_key(|s| (s.product_code.is_some() as u8, s.verified as u8))
    }

    pub fn by_id(&self, id: &str) -> Option<&Schema> {
        self.schemas.iter().find(|s| s.id == id)
    }

    #[cfg(feature = "serde")]
    pub fn load_json(&mut self, json: &str) -> Result<usize, String> {
        let v: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
        let list = if v.is_array() { v } else { serde_json::Value::Array(vec![v]) };
        let schemas: Vec<Schema> = serde_json::from_value(list).map_err(|e| e.to_string())?;
        let n = schemas.len();
        for s in schemas {
            self.add(s);
        }
        Ok(n)
    }

    #[cfg(feature = "serde")]
    pub fn load_dir(&mut self, dir: &std::path::Path) -> Vec<String> {
        let mut errors = Vec::new();
        if let Ok(rd) = std::fs::read_dir(dir) {
            let mut paths: Vec<_> = rd.flatten().map(|e| e.path()).collect();
            paths.sort();
            for p in paths {
                if p.is_dir() {
                    errors.extend(self.load_dir(&p));
                } else if p.extension().map(|e| e == "json").unwrap_or(false) {
                    match std::fs::read_to_string(&p).map_err(|e| e.to_string()).and_then(|s| self.load_json(&s)) {
                        Ok(_) => {}
                        Err(e) => errors.push(format!("{}: {}", p.display(), e)),
                    }
                }
            }
        }
        errors
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Schema {
        Schema {
            id: "test.breaker".into(),
            description: String::new(),
            manufacturer: 137,
            industry: 4,
            pgn: 126720,
            product_code: Some(12337),
            model: None,
            opcodes: vec![49],
            read: None,
            write: None,
            verified: false,
            source: "unit test".into(),
            fields: vec![
                SchemaField::uint("count", 8),
                SchemaField::group(
                    "breakers",
                    Count::Field("count".into()),
                    vec![
                        SchemaField::uint("channel", 4),
                        SchemaField::of(FieldKind::Bool, "locked", 1),
                        SchemaField::of(FieldKind::Reserved, "r", 3),
                        SchemaField::uint("rating", 16).scaled(0.1, "A"),
                        SchemaField::of(FieldKind::Int, "trim", 8),
                        SchemaField::of(FieldKind::StringLau, "label", 0),
                    ],
                ),
                SchemaField::of(FieldKind::StringFix, "tag", 32),
            ],
        }
    }

    #[test]
    fn roundtrip() {
        let s = sample();
        let rec: Record = vec![
            ("count".into(), Value::Int(2)),
            (
                "breakers".into(),
                Value::List(vec![
                    vec![
                        ("channel".into(), Value::Int(1)),
                        ("locked".into(), Value::Bool(true)),
                        ("rating".into(), Value::Float(12.5)),
                        ("trim".into(), Value::Int(-3)),
                        ("label".into(), Value::Text("BILGE".into())),
                    ],
                    vec![
                        ("channel".into(), Value::Int(15)),
                        ("locked".into(), Value::Bool(false)),
                        ("rating".into(), Value::Float(6.0)),
                        ("trim".into(), Value::Int(4)),
                        ("label".into(), Value::Text("NAV LIGHTS".into())),
                    ],
                ]),
            ),
            ("tag".into(), Value::Text("AB".into())),
        ];
        let bytes = s.encode(&rec).unwrap();
        let back = s.decode(&bytes).unwrap();
        assert_eq!(s.encode(&back).unwrap(), bytes);
        let Value::List(items) = &back[1].1 else { panic!() };
        assert_eq!(items[0][2].1, Value::Float(12.5));
        assert_eq!(items[1][3].1, Value::Int(4));
        assert_eq!(items[1][4].1, Value::Text("NAV LIGHTS".into()));
        assert_eq!(back[2].1, Value::Text("AB".into()));
    }

    #[test]
    fn range_checked() {
        let mut s = sample();
        s.fields[0].max = Some(10.0);
        assert!(matches!(s.encode(&vec![("count".into(), Value::Int(11))]), Err(CodecError::Range(_))));
    }

    #[test]
    fn registry_prefers_specific() {
        let mut r = SchemaRegistry::default();
        let mut generic = sample();
        generic.id = "generic".into();
        generic.product_code = None;
        r.add(generic);
        r.add(sample());
        assert_eq!(r.find(137, 126720, Some(12337), Some(49)).unwrap().id, "test.breaker");
        assert_eq!(r.find(137, 126720, Some(1), Some(49)).unwrap().id, "generic");
        assert!(r.find(137, 126720, Some(1), Some(48)).is_none());
    }
}

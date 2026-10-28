//! Generic NMEA 2000 decoder driven by the CANboat PGN database
//! (`canboat.json`, Apache-2.0, https://github.com/canboat/canboat).

use std::collections::HashMap;

use crate::bits::{get_bits, max_unsigned, sign_extend};
use crate::iso::{string_fix, string_lau};
use crate::value::{Field, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum PgnType {
    Single,
    Fast,
    Iso,
    Mixed,
}

#[derive(Clone, Debug, Default)]
pub struct FieldDef {
    pub order: usize,
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub bit_length: Option<usize>,
    pub bit_length_field: Option<usize>,
    pub field_type: String,
    pub signed: bool,
    pub resolution: f64,
    pub offset: i64,
    pub unit: Option<String>,
    pub lookup: Option<String>,
    pub bit_lookup: Option<String>,
    pub indirect_lookup: Option<String>,
    pub indirect_order: Option<usize>,
    pub match_value: Option<i64>,
    pub unknown_value: Option<i64>,
    pub out_of_range_value: Option<i64>,
    pub reserved_value: Option<i64>,
    pub proprietary_only: bool,
    pub primary_key: bool,
}

#[derive(Clone, Debug)]
pub struct Repeat {
    pub size: usize,
    pub start: usize,
    pub count_field: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct PgnDef {
    pub pgn: u32,
    pub id: String,
    pub description: String,
    pub kind: PgnType,
    pub priority: Option<u8>,
    pub length: Option<usize>,
    pub interval_ms: Option<u32>,
    pub fallback: bool,
    pub complete: bool,
    pub fields: Vec<FieldDef>,
    pub repeat1: Option<Repeat>,
    pub repeat2: Option<Repeat>,
}

#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Decoded {
    pub pgn: u32,
    pub id: String,
    pub description: String,
    pub fields: Vec<Field>,
    /// Set when the PGN is not in the database or only a fallback matched.
    pub unknown: bool,
}

impl Decoded {
    pub fn summary(&self, max_fields: usize) -> String {
        let mut parts = Vec::new();
        for f in &self.fields {
            if parts.len() >= max_fields {
                break;
            }
            if matches!(f.value, Value::Null) || f.id.starts_with("reserved") || f.id.starts_with("spare") {
                continue;
            }
            if matches!(f.id.as_str(), "manufacturerCode" | "industryCode" | "sid") {
                continue;
            }
            if let Value::List(l) = &f.value {
                parts.push(format!("{}: {} items", f.name, l.len()));
                continue;
            }
            parts.push(format!("{}: {}", f.name, f.friendly()));
        }
        parts.join(", ")
    }

    pub fn get(&self, id: &str) -> Option<&Value> {
        self.fields.iter().find(|f| f.id == id).map(|f| &f.value)
    }
}

impl PgnDb {
    /// Primary-key signature of a decoded message (fields flagged
    /// PartOfPrimaryKey, e.g. instance + temperature source). `None` when the
    /// PGN has no primary key (no instancing).
    pub fn primary_key(&self, decoded: &Decoded) -> Option<String> {
        let def = self.by_pgn.get(&decoded.pgn)?.iter().find(|d| d.id == decoded.id)?;
        let ids: Vec<&str> = def.fields.iter().filter(|f| f.primary_key).map(|f| f.id.as_str()).collect();
        if ids.is_empty() {
            return None;
        }
        let parts: Vec<String> = ids
            .iter()
            .filter_map(|id| decoded.fields.iter().find(|f| f.id == *id))
            // Instances are shown as numbers even where CANboat names them
            // (engine instance 0 = "Single Engine or Dual Engine Port").
            .map(|f| if f.id.to_ascii_lowercase().contains("instance") { format!("{}={}", f.name, f.value.display()) } else { format!("{}={}", f.name, f.text.clone().unwrap_or_else(|| f.value.display())) })
            .collect();
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(", "))
        }
    }
}

#[derive(Default)]
pub struct PgnDb {
    pub by_pgn: HashMap<u32, Vec<PgnDef>>,
    pub lookups: HashMap<String, HashMap<i64, String>>,
    pub bit_lookups: HashMap<String, HashMap<u32, String>>,
    pub indirect: HashMap<String, HashMap<(i64, i64), String>>,
    pub version: String,
}

pub const EMBEDDED_CANBOAT_JSON: &str = include_str!("../schemas/canboat.json");
/// Supplementary definitions not (yet) in CANboat, with their sources noted inside.
pub const EMBEDDED_COMMUNITY_JSON: &str = include_str!("../schemas/community.json");

impl PgnDb {
    pub fn get(&self, pgn: u32) -> Option<&[PgnDef]> {
        self.by_pgn.get(&pgn).map(|v| v.as_slice())
    }

    pub fn name(&self, pgn: u32) -> Option<&str> {
        self.by_pgn.get(&pgn).and_then(|v| v.iter().find(|d| !d.fallback).or(v.first())).map(|d| d.description.as_str())
    }

    pub fn is_fast(&self, pgn: u32) -> bool {
        self.by_pgn
            .get(&pgn)
            .map(|v| v.iter().any(|d| d.kind == PgnType::Fast))
            .unwrap_or_else(|| crate::fastpacket::builtin_fast_packet(pgn))
    }

    pub fn fast_pgns(&self) -> Vec<u32> {
        self.by_pgn.iter().filter(|(_, v)| v.iter().any(|d| d.kind == PgnType::Fast)).map(|(k, _)| *k).collect()
    }

    pub fn lookup_text(&self, table: &str, v: i64) -> Option<&str> {
        self.lookups.get(table).and_then(|t| t.get(&v)).map(|s| s.as_str())
    }

    pub fn manufacturer(&self, code: u16) -> Option<&str> {
        self.lookup_text("MANUFACTURER_CODE", code as i64)
    }

    pub fn device_class(&self, class: u8) -> Option<&str> {
        self.lookup_text("DEVICE_CLASS", class as i64)
    }

    pub fn device_function(&self, class: u8, function: u8) -> Option<&str> {
        self.indirect.get("DEVICE_FUNCTION").and_then(|t| t.get(&(class as i64, function as i64))).map(|s| s.as_str())
    }

    /// Byte width of field `field` (1-based order) of the main definition of `pgn`.
    pub fn field_bytes(&self, pgn: u32, field: u8, data: &[u8]) -> Option<usize> {
        let def = self.by_pgn.get(&pgn)?.iter().find(|d| !d.fallback)?;
        let f = def.fields.iter().find(|f| f.order == field as usize)?;
        match f.field_type.as_str() {
            "STRING_LAU" => data.first().map(|&l| (l as usize).max(2)),
            "STRING_LZ" => data.first().map(|&l| l as usize + 2),
            _ => f.bit_length.map(|b| b.div_ceil(8)),
        }
    }

    /// Encode a field value for a 126208 Command/Request parameter: the field's
    /// own width in whole bytes, little-endian, in raw units
    /// (`raw = value / resolution - offset`). Lookup fields take the numeric code.
    pub fn encode_param(&self, pgn: u32, order: u8, value: f64) -> Result<Vec<u8>, String> {
        let def = self.by_pgn.get(&pgn).and_then(|d| d.iter().find(|d| !d.fallback)).ok_or_else(|| format!("PGN {pgn} is not in the database"))?;
        let f = def.fields.iter().find(|f| f.order == order as usize).ok_or_else(|| format!("PGN {pgn} has no field {order}"))?;
        let bits = f.bit_length.ok_or_else(|| format!("{} has a variable length and cannot be set here", f.name))?;
        if bits > 32 || f.field_type.starts_with("STRING") || f.field_type == "BINARY" && bits > 8 {
            return Err(format!("{} cannot be set with this tool", f.name));
        }
        let res = if f.resolution == 0.0 { 1.0 } else { f.resolution };
        let raw = (value / res).round() as i64 - f.offset;
        // Top codes are reserved for "not available" / "out of range" in fields of 4+ bits.
        let reserve = if bits >= 4 { 2 } else { 0 };
        let (min, max) = if f.signed {
            (-(1i64 << (bits - 1)), (1i64 << (bits - 1)) - 1 - reserve)
        } else {
            (0, (1i64 << bits) - 1 - reserve)
        };
        if raw < min || raw > max {
            let lo = (min + f.offset) as f64 * res;
            let hi = (max + f.offset) as f64 * res;
            return Err(format!("{} must be between {lo} and {hi}{}", f.name, f.unit.as_deref().map(|u| format!(" {u}")).unwrap_or_default()));
        }
        let n = bits.div_ceil(8);
        Ok((raw as u64).to_le_bytes()[..n].to_vec())
    }

    /// Decode a full message payload.
    pub fn decode(&self, pgn: u32, data: &[u8]) -> Decoded {
        let defs = match self.by_pgn.get(&pgn) {
            Some(d) if !d.is_empty() => d,
            _ => {
                return Decoded {
                    pgn,
                    id: format!("pgn{pgn}"),
                    description: format!("Unknown PGN {pgn}"),
                    fields: vec![Field { id: "data".into(), name: "Data".into(), value: Value::Bytes(data.to_vec()), unit: None, text: None }],
                    unknown: true,
                }
            }
        };
        let def = self.select(defs, data);
        let mut fields = self.decode_with(def, data);
        if pgn == crate::iso::PGN_GROUP_FUNCTION {
            self.annotate_group_function(data, &mut fields);
        }
        Decoded { pgn, id: def.id.clone(), description: def.description.clone(), fields, unknown: def.fallback }
    }

    fn select<'a>(&self, defs: &'a [PgnDef], data: &[u8]) -> &'a PgnDef {
        let proprietary = crate::proprietary::is_proprietary_pgn(defs[0].pgn);
        let mut best: Option<(&PgnDef, usize)> = None;
        for d in defs.iter().filter(|d| !d.fallback) {
            let mut bit = 0usize;
            let mut ok = true;
            let mut matched = 0;
            for f in &d.fields {
                if f.proprietary_only && !proprietary {
                    continue;
                }
                let Some(len) = f.bit_length else { break };
                if let Some(m) = f.match_value {
                    match get_bits(data, bit, len) {
                        Some(v) if v as i64 == m => matched += 1,
                        _ => {
                            ok = false;
                            break;
                        }
                    }
                }
                bit += len;
            }
            if ok && best.map(|(_, m)| matched > m).unwrap_or(true) {
                best = Some((d, matched));
            }
        }
        best.map(|(d, _)| d).or_else(|| defs.iter().find(|d| d.fallback)).unwrap_or(&defs[0])
    }

    fn decode_with(&self, def: &PgnDef, data: &[u8]) -> Vec<Field> {
        let proprietary = crate::proprietary::is_proprietary_pgn(def.pgn);
        let mut out: Vec<Field> = Vec::new();
        let mut bit = 0usize;
        let mut raw_by_order: HashMap<usize, i64> = HashMap::new();
        let total_bits = data.len() * 8;
        let mut order_idx = 0usize;
        let fields = &def.fields;
        while order_idx < fields.len() {
            let f = &fields[order_idx];
            // Start of a repeating set?
            let rep = [&def.repeat1, &def.repeat2].into_iter().flatten().find(|r| r.start == f.order);
            if let Some(rep) = rep {
                {
                    let count = rep.count_field.and_then(|c| raw_by_order.get(&c).copied()).map(|c| c.max(0) as usize);
                    let group: Vec<&FieldDef> = fields.iter().filter(|x| x.order >= rep.start && x.order < rep.start + rep.size).collect();
                    let mut items = Vec::new();
                    let mut n = 0usize;
                    loop {
                        if let Some(c) = count {
                            if n >= c {
                                break;
                            }
                        }
                        if bit >= total_bits || n > 255 {
                            break;
                        }
                        let mut item = Vec::new();
                        for g in &group {
                            if let Some((field, used)) = self.decode_field(g, data, bit, &raw_by_order, proprietary) {
                                bit += used;
                                if let Some(fl) = field {
                                    item.push(fl);
                                }
                            } else {
                                bit = total_bits;
                                break;
                            }
                        }
                        if item.is_empty() {
                            break;
                        }
                        items.push(item.into_iter().map(|fl| (fl.id.clone(), fl.value.clone())).collect::<Vec<_>>());
                        n += 1;
                    }
                    out.push(Field { id: "list".into(), name: "Repeating fields".into(), value: Value::List(items), unit: None, text: None });
                    order_idx += rep.size.max(1);
                    continue;
                }
            }
            if bit >= total_bits && f.bit_length.unwrap_or(1) > 0 {
                break;
            }
            match self.decode_field(f, data, bit, &raw_by_order, proprietary) {
                Some((field, used)) => {
                    bit += used;
                    if let Some(fl) = field {
                        if let Value::Int(i) = fl.value {
                            raw_by_order.insert(f.order, i);
                        } else if let Some(r) = f.bit_length.and_then(|l| get_bits(data, bit - used, l.min(64))) {
                            raw_by_order.insert(f.order, r as i64);
                        }
                        out.push(fl);
                    } else if let Some(l) = f.bit_length {
                        if let Some(r) = get_bits(data, bit - used, l.min(64)) {
                            raw_by_order.insert(f.order, r as i64);
                        }
                    }
                }
                None => break,
            }
            order_idx += 1;
        }
        out
    }

    /// Returns (field or None if hidden, bits consumed); None if data exhausted.
    fn decode_field(&self, f: &FieldDef, data: &[u8], bit: usize, raw: &HashMap<usize, i64>, proprietary: bool) -> Option<(Option<Field>, usize)> {
        if f.proprietary_only && !proprietary {
            return Some((None, 0));
        }
        let total = data.len() * 8;
        let mk = |value: Value, text: Option<String>| Field { id: f.id.clone(), name: f.name.clone(), value, unit: f.unit.clone(), text };
        match f.field_type.as_str() {
            "STRING_LAU" => {
                let start = bit.div_ceil(8);
                let (s, n) = string_lau(data.get(start..)?)?;
                Some((Some(mk(Value::Text(s), None)), (start * 8 - bit) + n * 8))
            }
            "STRING_LZ" => {
                let start = bit.div_ceil(8);
                let b = data.get(start..)?;
                if let Some(bits) = f.bit_length {
                    let len = (bits / 8).min(b.len());
                    return Some((Some(mk(Value::Text(string_fix(&b[..len])), None)), bits));
                }
                let len = *b.first()? as usize;
                let body = b.get(1..(1 + len).min(b.len())).unwrap_or(&[]);
                let used = (1 + len + 1).min(b.len());
                Some((Some(mk(Value::Text(string_fix(body)), None)), (start * 8 - bit) + used * 8))
            }
            "STRING_FIX" => {
                let start = bit / 8;
                let len = f.bit_length.map(|b| b / 8).unwrap_or(data.len().saturating_sub(start));
                let end = (start + len).min(data.len());
                Some((Some(mk(Value::Text(string_fix(data.get(start..end)?)), None)), len * 8))
            }
            "BINARY" | "VARIABLE" | "DYNAMIC_FIELD_VALUE" | "KEY_VALUE" => {
                let bits = f
                    .bit_length
                    .or_else(|| f.bit_length_field.and_then(|o| raw.get(&o)).map(|v| *v as usize))
                    .unwrap_or(total.saturating_sub(bit));
                if bits == 0 {
                    return Some((Some(mk(Value::Bytes(vec![]), None)), 0));
                }
                let start = bit / 8;
                let end = (bit + bits).div_ceil(8).min(data.len());
                let bytes = data.get(start..end)?.to_vec();
                Some((Some(mk(Value::Bytes(bytes), None)), bits.min(total.saturating_sub(bit))))
            }
            "RESERVED" | "SPARE" => {
                let len = f.bit_length.unwrap_or(0);
                Some((None, len.min(total.saturating_sub(bit))))
            }
            "FLOAT" => {
                let v = get_bits(data, bit, 32)?;
                let fv = f32::from_bits(v as u32);
                let val = if v == 0xFFFF_FFFF || !fv.is_finite() { Value::Null } else { Value::Float(fv as f64) };
                Some((Some(mk(val, None)), 32))
            }
            "ISO_NAME" => {
                let v = get_bits(data, bit, 64)?;
                Some((Some(mk(Value::Text(format!("{:016X}", v)), None)), 64))
            }
            _ => {
                let len = f.bit_length?;
                if len > 64 {
                    let start = bit / 8;
                    let end = ((bit + len) / 8).min(data.len());
                    return Some((Some(mk(Value::Bytes(data.get(start..end)?.to_vec()), None)), len));
                }
                let r = get_bits(data, bit, len)?;
                let signed_val = if f.signed { sign_extend(r, len) } else { r as i64 };
                let is_lookup = matches!(f.field_type.as_str(), "LOOKUP" | "INDIRECT_LOOKUP" | "BITLOOKUP" | "FIELDTYPE_LOOKUP");
                let special = |x: Option<i64>| x.map(|u| u == signed_val).unwrap_or(false);
                if !is_lookup && (special(f.unknown_value) || (f.unknown_value.is_none() && len >= 2 && !f.signed && r == max_unsigned(len) && f.field_type != "PGN")) {
                    return Some((Some(mk(Value::Null, None)), len));
                }
                if !is_lookup && (special(f.out_of_range_value) || special(f.reserved_value)) {
                    return Some((Some(mk(Value::Null, Some("out of range".into()))), len));
                }
                let (value, text) = match f.field_type.as_str() {
                    "LOOKUP" => {
                        let t = f.lookup.as_ref().and_then(|l| self.lookup_text(l, signed_val)).map(|s| s.to_string());
                        (Value::Int(signed_val), t)
                    }
                    "INDIRECT_LOOKUP" => {
                        let other = f.indirect_order.and_then(|o| raw.get(&o)).copied().unwrap_or(-1);
                        let t = f
                            .indirect_lookup
                            .as_ref()
                            .and_then(|l| self.indirect.get(l))
                            .and_then(|t| t.get(&(other, signed_val)))
                            .cloned();
                        (Value::Int(signed_val), t)
                    }
                    "BITLOOKUP" => {
                        let names: Vec<String> = f
                            .bit_lookup
                            .as_ref()
                            .and_then(|l| self.bit_lookups.get(l))
                            .map(|t| (0..len).filter(|b| r >> b & 1 == 1).filter_map(|b| t.get(&(b as u32)).cloned()).collect())
                            .unwrap_or_default();
                        (Value::Int(signed_val), if names.is_empty() { None } else { Some(names.join(", ")) })
                    }
                    "DATE" => {
                        let t = days_to_date(signed_val);
                        (Value::Int(signed_val), Some(t))
                    }
                    "TIME" => {
                        let secs = signed_val as f64 * if f.resolution != 0.0 { f.resolution } else { 0.0001 };
                        let s = secs.max(0.0);
                        let t = format!("{:02}:{:02}:{:06.3}", (s / 3600.0) as u64, ((s % 3600.0) / 60.0) as u64, s % 60.0);
                        (Value::Float(secs), Some(t))
                    }
                    "MMSI" => (Value::Int(signed_val), Some(format!("{:09}", signed_val))),
                    "DECIMAL" => {
                        // BCD digits
                        let mut s = String::new();
                        for i in (0..len / 4).rev() {
                            s.push(char::from_digit(((r >> (i * 4)) & 0xF) as u32, 10).unwrap_or('?'));
                        }
                        (Value::Text(s), None)
                    }
                    "PGN" | "ADDRESS" | "FIELD_INDEX" | "DYNAMIC_FIELD_KEY" | "DYNAMIC_FIELD_LENGTH" => (Value::Int(signed_val), None),
                    _ => {
                        let raw_v = signed_val + f.offset;
                        if f.resolution == 1.0 || f.resolution == 0.0 {
                            (Value::Int(raw_v), None)
                        } else {
                            (Value::Float(raw_v as f64 * f.resolution), None)
                        }
                    }
                };
                Some((Some(mk(value, text)), len))
            }
        }
    }

    fn annotate_group_function(&self, data: &[u8], fields: &mut Vec<Field>) {
        let width = |pgn: u32, field: u8, rest: &[u8]| self.field_bytes(pgn, field, rest);
        if let Some(g) = crate::groupfunction::GroupFunction::decode(data, &width) {
            let target = self.name(g.pgn()).unwrap_or("?").to_string();
            fields.push(Field { id: "targetPgnName".into(), name: "Target PGN".into(), value: Value::Text(target), unit: None, text: None });
            if let crate::groupfunction::GroupFunction::Ack { pgn_error, interval_error, param_errors, .. } = g {
                let mut t = format!("PGN: {}, interval: {}", crate::groupfunction::pgn_error_text(pgn_error), crate::groupfunction::interval_error_text(interval_error));
                for (i, e) in param_errors.iter().enumerate() {
                    t.push_str(&format!(", param{}: {}", i + 1, crate::groupfunction::param_error_text(*e)));
                }
                fields.push(Field { id: "ackText".into(), name: "Result".into(), value: Value::Text(t), unit: None, text: None });
            }
        }
    }

    #[cfg(feature = "serde")]
    pub fn from_json(json: &str) -> Result<Self, String> {
        use serde_json::Value as J;
        let root: J = serde_json::from_str(json).map_err(|e| e.to_string())?;
        let mut db = PgnDb { version: root.get("Version").and_then(|v| v.as_str()).unwrap_or("").to_string(), ..Default::default() };
        let s = |v: &J, k: &str| v.get(k).and_then(|x| x.as_str()).map(|x| x.to_string());
        let n = |v: &J, k: &str| v.get(k).and_then(|x| x.as_i64().or_else(|| x.as_f64().map(|f| f as i64)));
        if let Some(arr) = root.get("LookupEnumerations").and_then(|v| v.as_array()) {
            for e in arr {
                let mut t = HashMap::new();
                for v in e.get("EnumValues").and_then(|v| v.as_array()).into_iter().flatten() {
                    if let (Some(name), Some(val)) = (s(v, "Name"), n(v, "Value")) {
                        t.insert(val, name);
                    }
                }
                if let Some(name) = s(e, "Name") {
                    db.lookups.insert(name, t);
                }
            }
        }
        if let Some(arr) = root.get("LookupBitEnumerations").and_then(|v| v.as_array()) {
            for e in arr {
                let mut t = HashMap::new();
                for v in e.get("EnumBitValues").and_then(|v| v.as_array()).into_iter().flatten() {
                    if let (Some(name), Some(val)) = (s(v, "Name"), n(v, "Bit")) {
                        t.insert(val as u32, name);
                    }
                }
                if let Some(name) = s(e, "Name") {
                    db.bit_lookups.insert(name, t);
                }
            }
        }
        if let Some(arr) = root.get("LookupIndirectEnumerations").and_then(|v| v.as_array()) {
            for e in arr {
                let mut t = HashMap::new();
                for v in e.get("EnumValues").and_then(|v| v.as_array()).into_iter().flatten() {
                    if let (Some(name), Some(a), Some(b)) = (s(v, "Name"), n(v, "Value1"), n(v, "Value2")) {
                        t.insert((a, b), name);
                    }
                }
                if let Some(name) = s(e, "Name") {
                    db.indirect.insert(name, t);
                }
            }
        }
        for p in root.get("PGNs").and_then(|v| v.as_array()).ok_or("missing PGNs")? {
            let pgn = n(p, "PGN").ok_or("PGN without number")? as u32;
            let kind = match s(p, "Type").as_deref() {
                Some("Fast") => PgnType::Fast,
                Some("ISO") => PgnType::Iso,
                Some("Mixed") => PgnType::Mixed,
                _ => PgnType::Single,
            };
            let mut fields = Vec::new();
            for f in p.get("Fields").and_then(|v| v.as_array()).into_iter().flatten() {
                fields.push(FieldDef {
                    order: n(f, "Order").unwrap_or(0) as usize,
                    id: s(f, "Id").unwrap_or_default(),
                    name: s(f, "Name").unwrap_or_default(),
                    description: s(f, "Description"),
                    bit_length: n(f, "BitLength").map(|x| x as usize),
                    bit_length_field: n(f, "BitLengthField").map(|x| x as usize),
                    field_type: s(f, "FieldType").unwrap_or_else(|| "NUMBER".into()),
                    signed: f.get("Signed").and_then(|x| x.as_bool()).unwrap_or(false),
                    resolution: f.get("Resolution").and_then(|x| x.as_f64()).unwrap_or(1.0),
                    offset: n(f, "Offset").unwrap_or(0),
                    unit: s(f, "Unit"),
                    lookup: s(f, "LookupEnumeration"),
                    bit_lookup: s(f, "LookupBitEnumeration"),
                    indirect_lookup: s(f, "LookupIndirectEnumeration"),
                    indirect_order: n(f, "LookupIndirectEnumerationFieldOrder").map(|x| x as usize),
                    match_value: n(f, "Match"),
                    unknown_value: n(f, "UnknownValue"),
                    out_of_range_value: n(f, "OutOfRangeValue"),
                    reserved_value: n(f, "ReservedValue"),
                    proprietary_only: s(f, "Condition").as_deref() == Some("PGNIsProprietary"),
                    primary_key: f.get("PartOfPrimaryKey").and_then(|x| x.as_bool()).unwrap_or(false),
                });
            }
            let rep = |i: u8| {
                let size = n(p, &format!("RepeatingFieldSet{i}Size"))? as usize;
                let start = n(p, &format!("RepeatingFieldSet{i}StartField"))? as usize;
                Some(Repeat { size, start, count_field: n(p, &format!("RepeatingFieldSet{i}CountField")).map(|x| x as usize) })
            };
            let def = PgnDef {
                pgn,
                id: s(p, "Id").unwrap_or_default(),
                description: s(p, "Description").unwrap_or_default(),
                kind,
                priority: n(p, "Priority").map(|x| x as u8),
                length: n(p, "Length").map(|x| x as usize),
                interval_ms: n(p, "TransmissionInterval").map(|x| x as u32),
                fallback: p.get("Fallback").and_then(|x| x.as_bool()).unwrap_or(false),
                complete: p.get("Complete").and_then(|x| x.as_bool()).unwrap_or(false),
                fields,
                repeat1: rep(1),
                repeat2: rep(2),
            };
            db.by_pgn.entry(pgn).or_default().push(def);
        }
        Ok(db)
    }

    /// Merge another database in CANboat JSON format. Its PGN definitions take
    /// precedence over existing ones with the same id; lookup values are added
    /// where missing.
    #[cfg(feature = "serde")]
    pub fn merge_json(&mut self, json: &str) -> Result<usize, String> {
        let other = Self::from_json(json)?;
        for (k, v) in other.lookups {
            let e = self.lookups.entry(k).or_default();
            for (a, b) in v {
                e.entry(a).or_insert(b);
            }
        }
        for (k, v) in other.bit_lookups {
            let e = self.bit_lookups.entry(k).or_default();
            for (a, b) in v {
                e.entry(a).or_insert(b);
            }
        }
        for (k, v) in other.indirect {
            let e = self.indirect.entry(k).or_default();
            for (a, b) in v {
                e.entry(a).or_insert(b);
            }
        }
        let mut n = 0;
        for (pgn, defs) in other.by_pgn {
            let e = self.by_pgn.entry(pgn).or_default();
            for d in defs.into_iter().rev() {
                e.retain(|x| x.id != d.id);
                e.insert(0, d);
                n += 1;
            }
        }
        Ok(n)
    }

    #[cfg(feature = "serde")]
    pub fn embedded() -> Self {
        let mut db = Self::from_json(EMBEDDED_CANBOAT_JSON).expect("embedded canboat.json is valid");
        db.merge_json(EMBEDDED_COMMUNITY_JSON).expect("embedded community.json is valid");
        db
    }
}

fn days_to_date(days: i64) -> String {
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{:04}-{:02}-{:02}", if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(days_to_date(0), "1970-01-01");
        assert_eq!(days_to_date(19_000), "2022-01-08");
    }

    #[cfg(feature = "serde")]
    mod db {
        use super::super::*;

        fn db() -> &'static PgnDb {
            use std::sync::OnceLock;
            static DB: OnceLock<PgnDb> = OnceLock::new();
            DB.get_or_init(PgnDb::embedded)
        }

        #[test]
        fn encode_params() {
            let db = db();
            // 127505 capacity: 0.1 L, 32 bits (byte-identical to the TLM100 tool)
            assert_eq!(db.encode_param(127505, 4, 400.0).unwrap(), 4000u32.to_le_bytes().to_vec());
            // 127505 fluid type: 4-bit lookup -> one byte
            assert_eq!(db.encode_param(127505, 2, 1.0).unwrap(), vec![1]);
            // 128267 offset: signed 0.001 m, 16 bits
            assert_eq!(db.encode_param(128267, 3, -0.5).unwrap(), (-500i16).to_le_bytes().to_vec());
            assert!(db.encode_param(127505, 1, 15.0).is_err()); // 4-bit instance: 14/15 reserved
        }

        #[test]
        fn heading_127250() {
            // SID 0, heading 1.2345 rad, deviation n/a, variation n/a, ref magnetic(1)
            let mut d = vec![0u8];
            d.extend_from_slice(&12345u16.to_le_bytes());
            d.extend_from_slice(&0x7FFFu16.to_le_bytes());
            d.extend_from_slice(&0x7FFFu16.to_le_bytes());
            d.push(0xFD); // reference = 1 (Magnetic) in 2 bits + reserved ones
            let r = db().decode(127250, &d);
            assert_eq!(r.description, "Vessel Heading");
            let h = r.get("heading").unwrap().as_f64().unwrap();
            assert!((h - 1.2345).abs() < 1e-9);
            assert_eq!(r.get("deviation"), Some(&Value::Null));
            let reference = r.fields.iter().find(|f| f.id == "reference").unwrap();
            assert_eq!(reference.text.as_deref(), Some("Magnetic"));
        }

        #[test]
        fn position_129025() {
            let lat: i32 = 523_456_789; // 52.3456789
            let lon: i32 = -41_234_567; // -4.1234567
            let mut d = lat.to_le_bytes().to_vec();
            d.extend_from_slice(&lon.to_le_bytes());
            let r = db().decode(129025, &d);
            assert!((r.get("latitude").unwrap().as_f64().unwrap() - 52.3456789).abs() < 1e-7);
            assert!((r.get("longitude").unwrap().as_f64().unwrap() + 4.1234567).abs() < 1e-7);
        }

        #[test]
        fn product_info_and_address_claim() {
            let p = crate::iso::ProductInfo { model_id: "GPS200".into(), software_version: "1.2".into(), nmea2000_version: Some(2100), product_code: Some(3373), ..Default::default() };
            let r = db().decode(126996, &p.encode());
            assert_eq!(r.get("modelId").unwrap().as_str(), Some("GPS200"));
            let n = crate::iso::IsoName { manufacturer_code: 137, device_class: 60, device_function: 145, industry_group: 4, ..Default::default() };
            let r = db().decode(60928, &n.to_bytes());
            let m = r.fields.iter().find(|f| f.id == "manufacturerCode").unwrap();
            assert_eq!(m.text.as_deref(), Some("Maretron"));
            assert!(db().device_class(60).is_some());
        }

        #[test]
        fn maretron_proprietary_selected_by_match() {
            let m = crate::proprietary::maretron::ConfigMessage { product_code: 12337, software_code: 1, opcode: 2, payload: vec![1, 2] };
            let r = db().decode(126720, &m.encode());
            assert_eq!(r.id, "maretronProprietaryConfiguration");
        }

        #[test]
        fn group_function_ack_annotated() {
            let a = crate::groupfunction::GroupFunction::Ack { pgn: 126998, pgn_error: 0, interval_error: 0, param_errors: vec![0] };
            let r = db().decode(126208, &a.encode());
            assert!(r.summary(10).contains("Acknowledge"), "{}", r.summary(10));
        }

        #[test]
        fn community_maretron_responses() {
            use crate::proprietary::maretron::ConfigMessage;
            // Variation source reply from an SSC300: source NMEA 2000, variation -0.12 rad
            let mut payload = vec![1u8];
            payload.extend_from_slice(&(-1200i16).to_le_bytes());
            let m = ConfigMessage { product_code: 2686, software_code: 1, opcode: 93, payload };
            let r = db().decode(126720, &m.encode());
            assert_eq!(r.id, "maretronVariationSourceResponse", "{:?}", r.id);
            assert_eq!(r.fields.iter().find(|f| f.id == "source").unwrap().text.as_deref(), Some("NMEA 2000"));
            // Installation configuration (sub-command 513) vs damping (sub-command 1)
            let mut p = 513u16.to_le_bytes().to_vec();
            p.extend_from_slice(&[2, 0, 1, 0]);
            let r = db().decode(126720, &ConfigMessage { product_code: 2686, software_code: 1, opcode: 94, payload: p }.encode());
            assert_eq!(r.id, "maretronInstallationConfigurationResponse");
            let mut p = 1u16.to_le_bytes().to_vec();
            p.extend_from_slice(&1500u16.to_le_bytes());
            let r = db().decode(126720, &ConfigMessage { product_code: 2686, software_code: 1, opcode: 94, payload: p }.encode());
            assert_eq!(r.id, "maretronRotDampingGetResponse");
            assert!((r.get("dampingPeriod").unwrap().as_f64().unwrap() - 1.5).abs() < 1e-9);
            assert_eq!(db().lookup_text("MARETRON_PRODUCT_CODE", 27725), Some("TLM100"));
        }

        #[test]
        fn fast_list_from_db() {
            assert!(db().is_fast(129029));
            assert!(!db().is_fast(127250));
            assert_eq!(db().field_bytes(60928, 3, &[]), Some(1));
        }
    }
}

//! PGN 126208 NMEA Group Function encode/decode.

use crate::proprietary::{is_proprietary_pgn, ProprietaryHeader};

pub const FN_REQUEST: u8 = 0;
pub const FN_COMMAND: u8 = 1;
pub const FN_ACK: u8 = 2;
pub const FN_READ: u8 = 3;
pub const FN_READ_REPLY: u8 = 4;
pub const FN_WRITE: u8 = 5;
pub const FN_WRITE_REPLY: u8 = 6;

pub const INTERVAL_UNCHANGED: u32 = 0xFFFF_FFFF;
pub const INTERVAL_RESTORE_DEFAULT: u32 = 0xFFFF_FFFE;
pub const OFFSET_UNCHANGED: u16 = 0xFFFF;
pub const PRIORITY_UNCHANGED: u8 = 8;

/// (field number, raw little-endian value bytes)
pub type Param = (u8, Vec<u8>);

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum GroupFunction {
    Request { pgn: u32, interval_ms: u32, offset_10ms: u16, params: Vec<Param> },
    Command { pgn: u32, priority: u8, params: Vec<Param> },
    Ack { pgn: u32, pgn_error: u8, interval_error: u8, param_errors: Vec<u8> },
    Read { pgn: u32, header: Option<ProprietaryHeader>, unique_id: u8, selection: Vec<Param>, fields: Vec<u8> },
    ReadReply { pgn: u32, header: Option<ProprietaryHeader>, unique_id: u8, selection: Vec<Param>, values: Vec<Param> },
    Write { pgn: u32, header: Option<ProprietaryHeader>, unique_id: u8, selection: Vec<Param>, values: Vec<Param> },
    WriteReply { pgn: u32, header: Option<ProprietaryHeader>, unique_id: u8, selection: Vec<Param>, values: Vec<Param> },
}

pub fn pgn_error_text(code: u8) -> &'static str {
    match code {
        0 => "Acknowledge",
        1 => "PGN not supported",
        2 => "PGN not available",
        3 => "Access denied",
        4 => "Not supported",
        5 => "Tag not supported",
        6 => "Read or write not supported",
        _ => "Unknown error",
    }
}

pub fn param_error_text(code: u8) -> &'static str {
    match code {
        0 => "Acknowledge",
        1 => "Invalid parameter field",
        2 => "Temporary error",
        3 => "Parameter out of range",
        4 => "Access denied",
        5 => "Not supported",
        6 => "Read or write not supported",
        _ => "Unknown error",
    }
}

pub fn interval_error_text(code: u8) -> &'static str {
    match code {
        0 => "Acknowledge",
        1 => "Transmit interval/priority not supported",
        2 => "Transmit interval too low",
        3 => "Access denied",
        4 => "Not supported",
        _ => "Unknown error",
    }
}

fn put_pgn(v: &mut Vec<u8>, pgn: u32) {
    v.extend_from_slice(&pgn.to_le_bytes()[..3]);
}

fn get_pgn(b: &[u8], o: usize) -> Option<u32> {
    Some(*b.get(o)? as u32 | (*b.get(o + 1)? as u32) << 8 | (*b.get(o + 2)? as u32) << 16)
}

fn put_params(v: &mut Vec<u8>, ps: &[Param]) {
    for (f, val) in ps {
        v.push(*f);
        v.extend_from_slice(val);
    }
}

/// Resolves the byte width of field `field` (1-based) of `pgn`.
pub type WidthFn<'a> = &'a dyn Fn(u32, u8, &[u8]) -> Option<usize>;

fn get_params(b: &[u8], mut o: usize, n: usize, pgn: u32, width: WidthFn) -> (Vec<Param>, usize) {
    let mut out = Vec::new();
    for _ in 0..n {
        let Some(&f) = b.get(o) else { break };
        o += 1;
        let w = width(pgn, f, &b[o.min(b.len())..]).unwrap_or(b.len().saturating_sub(o));
        let end = (o + w).min(b.len());
        out.push((f, b[o..end].to_vec()));
        o = end;
    }
    (out, o)
}

impl GroupFunction {
    pub fn pgn(&self) -> u32 {
        match self {
            GroupFunction::Request { pgn, .. }
            | GroupFunction::Command { pgn, .. }
            | GroupFunction::Ack { pgn, .. }
            | GroupFunction::Read { pgn, .. }
            | GroupFunction::ReadReply { pgn, .. }
            | GroupFunction::Write { pgn, .. }
            | GroupFunction::WriteReply { pgn, .. } => *pgn,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut v = Vec::new();
        match self {
            GroupFunction::Request { pgn, interval_ms, offset_10ms, params } => {
                v.push(FN_REQUEST);
                put_pgn(&mut v, *pgn);
                v.extend_from_slice(&interval_ms.to_le_bytes());
                v.extend_from_slice(&offset_10ms.to_le_bytes());
                v.push(params.len() as u8);
                put_params(&mut v, params);
            }
            GroupFunction::Command { pgn, priority, params } => {
                v.push(FN_COMMAND);
                put_pgn(&mut v, *pgn);
                v.push((priority & 0x0F) | 0xF0);
                v.push(params.len() as u8);
                put_params(&mut v, params);
            }
            GroupFunction::Ack { pgn, pgn_error, interval_error, param_errors } => {
                v.push(FN_ACK);
                put_pgn(&mut v, *pgn);
                v.push((pgn_error & 0x0F) | (interval_error & 0x0F) << 4);
                v.push(param_errors.len() as u8);
                for pair in param_errors.chunks(2) {
                    let lo = pair[0] & 0x0F;
                    let hi = pair.get(1).copied().unwrap_or(0x0F) & 0x0F;
                    v.push(lo | hi << 4);
                }
            }
            GroupFunction::Read { pgn, header, unique_id, selection, fields } => {
                v.push(FN_READ);
                put_pgn(&mut v, *pgn);
                if let Some(h) = header {
                    v.extend_from_slice(&h.encode());
                }
                v.push(*unique_id);
                v.push(selection.len() as u8);
                v.push(fields.len() as u8);
                put_params(&mut v, selection);
                v.extend_from_slice(fields);
            }
            GroupFunction::ReadReply { pgn, header, unique_id, selection, values }
            | GroupFunction::Write { pgn, header, unique_id, selection, values }
            | GroupFunction::WriteReply { pgn, header, unique_id, selection, values } => {
                v.push(match self {
                    GroupFunction::ReadReply { .. } => FN_READ_REPLY,
                    GroupFunction::Write { .. } => FN_WRITE,
                    _ => FN_WRITE_REPLY,
                });
                put_pgn(&mut v, *pgn);
                if let Some(h) = header {
                    v.extend_from_slice(&h.encode());
                }
                v.push(*unique_id);
                v.push(selection.len() as u8);
                v.push(values.len() as u8);
                put_params(&mut v, selection);
                put_params(&mut v, values);
            }
        }
        v
    }

    pub fn decode(b: &[u8], width: WidthFn) -> Option<Self> {
        let f = *b.first()?;
        let pgn = get_pgn(b, 1)?;
        match f {
            FN_REQUEST => {
                let interval_ms = u32::from_le_bytes(b.get(4..8)?.try_into().ok()?);
                let offset_10ms = u16::from_le_bytes(b.get(8..10)?.try_into().ok()?);
                let n = *b.get(10)? as usize;
                let (params, _) = get_params(b, 11, n, pgn, width);
                Some(GroupFunction::Request { pgn, interval_ms, offset_10ms, params })
            }
            FN_COMMAND => {
                let priority = *b.get(4)? & 0x0F;
                let n = *b.get(5)? as usize;
                let (params, _) = get_params(b, 6, n, pgn, width);
                Some(GroupFunction::Command { pgn, priority, params })
            }
            FN_ACK => {
                let e = *b.get(4)?;
                let n = *b.get(5)? as usize;
                let mut param_errors = Vec::with_capacity(n);
                for i in 0..n {
                    let byte = b.get(6 + i / 2).copied().unwrap_or(0xFF);
                    param_errors.push(if i % 2 == 0 { byte & 0x0F } else { byte >> 4 });
                }
                Some(GroupFunction::Ack { pgn, pgn_error: e & 0x0F, interval_error: e >> 4, param_errors })
            }
            FN_READ | FN_READ_REPLY | FN_WRITE | FN_WRITE_REPLY => {
                let mut o = 4;
                let header = if is_proprietary_pgn(pgn) {
                    let h = ProprietaryHeader::decode(b.get(4..6)?)?;
                    o += 2;
                    Some(h)
                } else {
                    None
                };
                let unique_id = *b.get(o)?;
                let nsel = *b.get(o + 1)? as usize;
                let npar = *b.get(o + 2)? as usize;
                let (selection, o2) = get_params(b, o + 3, nsel, pgn, width);
                if f == FN_READ {
                    let fields = b.get(o2..o2 + npar)?.to_vec();
                    return Some(GroupFunction::Read { pgn, header, unique_id, selection, fields });
                }
                let (values, _) = get_params(b, o2, npar, pgn, width);
                Some(match f {
                    FN_READ_REPLY => GroupFunction::ReadReply { pgn, header, unique_id, selection, values },
                    FN_WRITE => GroupFunction::Write { pgn, header, unique_id, selection, values },
                    _ => GroupFunction::WriteReply { pgn, header, unique_id, selection, values },
                })
            }
            _ => None,
        }
    }
}

/// Convenience: unsigned little-endian value of `bytes` width.
pub fn le_value(v: u64, bytes: usize) -> Vec<u8> {
    v.to_le_bytes()[..bytes.min(8)].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(_: u32, f: u8, _: &[u8]) -> Option<usize> {
        Some(if f == 3 { 2 } else { 1 })
    }

    #[test]
    fn request_roundtrip() {
        let g = GroupFunction::Request {
            pgn: 127250,
            interval_ms: 1000,
            offset_10ms: OFFSET_UNCHANGED,
            params: vec![(1, vec![5]), (3, vec![0x34, 0x12])],
        };
        let e = g.encode();
        assert_eq!(GroupFunction::decode(&e, &w).unwrap(), g);
    }

    #[test]
    fn command_and_ack() {
        let g = GroupFunction::Command { pgn: 60928, priority: PRIORITY_UNCHANGED, params: vec![(3, vec![2, 0]), (4, vec![1])] };
        assert_eq!(GroupFunction::decode(&g.encode(), &w).unwrap(), g);
        let a = GroupFunction::Ack { pgn: 60928, pgn_error: 0, interval_error: 0, param_errors: vec![0, 3, 1] };
        assert_eq!(GroupFunction::decode(&a.encode(), &w).unwrap(), a);
    }

    #[test]
    fn proprietary_write() {
        let h = ProprietaryHeader { manufacturer_code: 137, industry_code: 4 };
        let g = GroupFunction::Write { pgn: 126720, header: Some(h), unique_id: 1, selection: vec![(1, vec![2])], values: vec![(3, vec![9, 9])] };
        assert_eq!(GroupFunction::decode(&g.encode(), &w).unwrap(), g);
    }
}

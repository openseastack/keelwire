//! PCAP / PCAPNG writing and reading with LINKTYPE_CAN_SOCKETCAN (227).

use std::io::{self, Read, Write};

use crate::frame::RawCanFrame;

pub const LINKTYPE_CAN_SOCKETCAN: u16 = 227;
const CAN_EFF_FLAG: u32 = 0x8000_0000;
const CAN_ERR_FLAG: u32 = 0x2000_0000;

/// 16-byte SocketCAN header + data as stored in captures.
pub fn socketcan_bytes(f: &RawCanFrame) -> [u8; 16] {
    let mut b = [0u8; 16];
    let id = f.can_id | if f.is_extended { CAN_EFF_FLAG } else { 0 };
    b[0..4].copy_from_slice(&id.to_be_bytes());
    b[4] = f.dlc.min(8);
    b[8..16].copy_from_slice(&f.data);
    b
}

pub fn parse_socketcan(b: &[u8], timestamp_us: u64) -> Option<RawCanFrame> {
    if b.len() < 8 {
        return None;
    }
    let raw = u32::from_be_bytes(b[0..4].try_into().ok()?);
    if raw & CAN_ERR_FLAG != 0 {
        return None;
    }
    let dlc = b[4].min(8);
    let mut data = [0xFFu8; 8];
    let n = (dlc as usize).min(b.len().saturating_sub(8));
    data[..n].copy_from_slice(&b[8..8 + n]);
    Some(RawCanFrame { timestamp_us, can_id: raw & 0x1FFF_FFFF, is_extended: raw & CAN_EFF_FLAG != 0, dlc, data })
}

/// Classic libpcap writer (used for Wireshark extcap FIFOs).
pub struct PcapWriter<W: Write> {
    w: W,
}

impl<W: Write> PcapWriter<W> {
    pub fn new(mut w: W) -> io::Result<Self> {
        w.write_all(&0xA1B2_C3D4u32.to_le_bytes())?;
        w.write_all(&2u16.to_le_bytes())?;
        w.write_all(&4u16.to_le_bytes())?;
        w.write_all(&0i32.to_le_bytes())?;
        w.write_all(&0u32.to_le_bytes())?;
        w.write_all(&65535u32.to_le_bytes())?;
        w.write_all(&(LINKTYPE_CAN_SOCKETCAN as u32).to_le_bytes())?;
        w.flush()?;
        Ok(PcapWriter { w })
    }

    /// `unix_us` = absolute timestamp in microseconds since the Unix epoch.
    pub fn write(&mut self, unix_us: u64, f: &RawCanFrame) -> io::Result<()> {
        let b = socketcan_bytes(f);
        self.w.write_all(&((unix_us / 1_000_000) as u32).to_le_bytes())?;
        self.w.write_all(&((unix_us % 1_000_000) as u32).to_le_bytes())?;
        self.w.write_all(&16u32.to_le_bytes())?;
        self.w.write_all(&16u32.to_le_bytes())?;
        self.w.write_all(&b)
    }

    pub fn flush(&mut self) -> io::Result<()> {
        self.w.flush()
    }
}

/// PCAPNG writer (default capture format).
pub struct PcapngWriter<W: Write> {
    w: W,
}

fn block(w: &mut impl Write, kind: u32, body: &[u8]) -> io::Result<()> {
    let pad = (4 - body.len() % 4) % 4;
    let total = (12 + body.len() + pad) as u32;
    w.write_all(&kind.to_le_bytes())?;
    w.write_all(&total.to_le_bytes())?;
    w.write_all(body)?;
    w.write_all(&[0u8; 3][..pad])?;
    w.write_all(&total.to_le_bytes())
}

fn option(buf: &mut Vec<u8>, code: u16, val: &[u8]) {
    buf.extend_from_slice(&code.to_le_bytes());
    buf.extend_from_slice(&(val.len() as u16).to_le_bytes());
    buf.extend_from_slice(val);
    buf.extend(std::iter::repeat_n(0, (4 - val.len() % 4) % 4));
}

impl<W: Write> PcapngWriter<W> {
    pub fn new(mut w: W, app: &str, if_name: &str) -> io::Result<Self> {
        let mut shb = Vec::new();
        shb.extend_from_slice(&0x1A2B_3C4Du32.to_le_bytes());
        shb.extend_from_slice(&1u16.to_le_bytes());
        shb.extend_from_slice(&0u16.to_le_bytes());
        shb.extend_from_slice(&(-1i64).to_le_bytes());
        option(&mut shb, 4, app.as_bytes()); // shb_userappl
        option(&mut shb, 0, &[]);
        block(&mut w, 0x0A0D_0D0A, &shb)?;
        let mut idb = Vec::new();
        idb.extend_from_slice(&LINKTYPE_CAN_SOCKETCAN.to_le_bytes());
        idb.extend_from_slice(&0u16.to_le_bytes());
        idb.extend_from_slice(&65535u32.to_le_bytes());
        option(&mut idb, 2, if_name.as_bytes()); // if_name
        option(&mut idb, 9, &[6]); // if_tsresol = 10^-6
        option(&mut idb, 0, &[]);
        block(&mut w, 1, &idb)?;
        w.flush()?;
        Ok(PcapngWriter { w })
    }

    pub fn write(&mut self, unix_us: u64, f: &RawCanFrame) -> io::Result<()> {
        let mut epb = Vec::with_capacity(36);
        epb.extend_from_slice(&0u32.to_le_bytes());
        epb.extend_from_slice(&((unix_us >> 32) as u32).to_le_bytes());
        epb.extend_from_slice(&(unix_us as u32).to_le_bytes());
        epb.extend_from_slice(&16u32.to_le_bytes());
        epb.extend_from_slice(&16u32.to_le_bytes());
        epb.extend_from_slice(&socketcan_bytes(f));
        block(&mut self.w, 6, &epb)
    }

    pub fn flush(&mut self) -> io::Result<()> {
        self.w.flush()
    }

    pub fn into_inner(self) -> W {
        self.w
    }
}

/// Read every CAN frame from a pcap or pcapng file. Timestamps are absolute
/// Unix microseconds.
pub fn read_all(mut r: impl Read) -> io::Result<Vec<RawCanFrame>> {
    let mut buf = Vec::new();
    r.read_to_end(&mut buf)?;
    parse_bytes(&buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn parse_bytes(buf: &[u8]) -> Result<Vec<RawCanFrame>, String> {
    if buf.len() < 24 {
        return Err("file too short".into());
    }
    let magic = u32::from_le_bytes(buf[0..4].try_into().unwrap());
    match magic {
        0x0A0D_0D0A => parse_pcapng(buf),
        0xA1B2_C3D4 | 0xA1B2_3C4D => parse_pcap(buf, false, magic == 0xA1B2_3C4D),
        0xD4C3_B2A1 | 0x4D3C_B2A1 => parse_pcap(buf, true, magic == 0x4D3C_B2A1),
        _ => Err("not a pcap/pcapng file".into()),
    }
}

fn rd32(b: &[u8], o: usize, be: bool) -> u32 {
    let a: [u8; 4] = b[o..o + 4].try_into().unwrap();
    if be {
        u32::from_be_bytes(a)
    } else {
        u32::from_le_bytes(a)
    }
}

fn parse_pcap(b: &[u8], be: bool, nanos: bool) -> Result<Vec<RawCanFrame>, String> {
    let linktype = rd32(b, 20, be);
    if linktype != LINKTYPE_CAN_SOCKETCAN as u32 {
        return Err(format!("unsupported link type {linktype} (need 227 CAN_SOCKETCAN)"));
    }
    let mut out = Vec::new();
    let mut o = 24;
    while o + 16 <= b.len() {
        let sec = rd32(b, o, be) as u64;
        let frac = rd32(b, o + 4, be) as u64;
        let incl = rd32(b, o + 8, be) as usize;
        o += 16;
        if o + incl > b.len() {
            break;
        }
        let us = sec * 1_000_000 + if nanos { frac / 1000 } else { frac };
        if let Some(f) = parse_socketcan(&b[o..o + incl], us) {
            out.push(f);
        }
        o += incl;
    }
    Ok(out)
}

fn parse_pcapng(b: &[u8]) -> Result<Vec<RawCanFrame>, String> {
    let mut out = Vec::new();
    let mut o = 0;
    let mut be = false;
    let mut ifaces: Vec<(u16, u64)> = Vec::new(); // (linktype, units per second)
    while o + 12 <= b.len() {
        let mut kind = rd32(b, o, be);
        if kind == 0x0A0D_0D0A {
            let bom = u32::from_le_bytes(b[o + 8..o + 12].try_into().unwrap());
            be = bom != 0x1A2B_3C4D;
            kind = 0x0A0D_0D0A;
            ifaces.clear();
        }
        let len = rd32(b, o + 4, be) as usize;
        if len < 12 || o + len > b.len() {
            break;
        }
        let body = &b[o + 8..o + len - 4];
        match kind {
            1 => {
                let lt = if be { u16::from_be_bytes([body[0], body[1]]) } else { u16::from_le_bytes([body[0], body[1]]) };
                let mut res = 1_000_000u64;
                let mut p = 8;
                while p + 4 <= body.len() {
                    let code = if be { u16::from_be_bytes([body[p], body[p + 1]]) } else { u16::from_le_bytes([body[p], body[p + 1]]) };
                    let l = if be { u16::from_be_bytes([body[p + 2], body[p + 3]]) } else { u16::from_le_bytes([body[p + 2], body[p + 3]]) } as usize;
                    if code == 0 {
                        break;
                    }
                    if code == 9 && l >= 1 {
                        let v = body[p + 4];
                        res = if v & 0x80 != 0 { 1u64 << (v & 0x7F) } else { 10u64.pow(v as u32) };
                    }
                    p += 4 + l + (4 - l % 4) % 4;
                }
                ifaces.push((lt, res));
            }
            6 => {
                let ifid = rd32(body, 0, be) as usize;
                let ts = ((rd32(body, 4, be) as u64) << 32) | rd32(body, 8, be) as u64;
                let caplen = rd32(body, 12, be) as usize;
                if let Some(&(lt, res)) = ifaces.get(ifid) {
                    if lt == LINKTYPE_CAN_SOCKETCAN && 20 + caplen <= body.len() {
                        let us = (ts as u128 * 1_000_000 / res as u128) as u64;
                        if let Some(f) = parse_socketcan(&body[20..20 + caplen], us) {
                            out.push(f);
                        }
                    }
                }
            }
            3 => {
                if let Some(&(lt, _)) = ifaces.first() {
                    if lt == LINKTYPE_CAN_SOCKETCAN && body.len() >= 4 + 16 {
                        if let Some(f) = parse_socketcan(&body[4..], 0) {
                            out.push(f);
                        }
                    }
                }
            }
            _ => {}
        }
        o += len;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames() -> Vec<RawCanFrame> {
        vec![
            RawCanFrame::new(1_700_000_000_000_001, 0x09F8_0117, &[1, 2, 3, 4, 5, 6, 7, 8]),
            RawCanFrame::new(1_700_000_000_500_000, 0x18EA_2380, &[0x14, 0xF0, 0x01]),
        ]
    }

    #[test]
    fn pcapng_roundtrip() {
        let mut w = PcapngWriter::new(Vec::new(), "test", "can0").unwrap();
        for f in frames() {
            w.write(f.timestamp_us, &f).unwrap();
        }
        let bytes = w.into_inner();
        assert_eq!(bytes.len() % 4, 0);
        assert_eq!(parse_bytes(&bytes).unwrap(), frames());
    }

    #[test]
    fn pcap_roundtrip() {
        let mut v = Vec::new();
        {
            let mut w = PcapWriter::new(&mut v).unwrap();
            for f in frames() {
                w.write(f.timestamp_us, &f).unwrap();
            }
        }
        assert_eq!(parse_bytes(&v).unwrap(), frames());
    }
}

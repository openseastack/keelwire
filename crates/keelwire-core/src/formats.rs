//! Text log / gateway line formats: Yacht Devices RAW, candump, CANboat
//! plain CSV and Actisense N2K ASCII.

use crate::frame::{hex, CanId, N2kMessage, RawCanFrame};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LineItem {
    Frame(RawCanFrame),
    Message(N2kMessage),
}

fn hexbytes<'a>(it: impl Iterator<Item = &'a str>) -> Option<Vec<u8>> {
    it.map(|t| u8::from_str_radix(t, 16).ok()).collect()
}

/// `hh:mm:ss.sss` → microseconds since midnight.
fn tod_us(s: &str) -> Option<u64> {
    let mut p = s.split(':');
    let h: u64 = p.next()?.parse().ok()?;
    let m: u64 = p.next()?.parse().ok()?;
    let sec: f64 = p.next()?.parse().ok()?;
    Some(h * 3_600_000_000 + m * 60_000_000 + (sec * 1_000_000.0).round() as u64)
}

/// Yacht Devices RAW: `17:33:21.107 R 19F51323 01 2F 30 70 00 2F 30 70`.
/// Returns (is_tx_echo, frame). Timestamp = µs since midnight.
pub fn parse_yd_raw(line: &str) -> Option<(bool, RawCanFrame)> {
    let mut it = line.split_whitespace();
    let t = it.next()?;
    let ts = tod_us(t)?;
    let dir = it.next()?;
    let tx = match dir {
        "R" => false,
        "T" => true,
        _ => return None,
    };
    let id = u32::from_str_radix(it.next()?, 16).ok()?;
    let data = hexbytes(it)?;
    if data.len() > 8 {
        return None;
    }
    Some((tx, RawCanFrame::new(ts, id, &data)))
}

/// Line to send to a YD gateway to transmit a frame.
pub fn yd_raw_tx(f: &RawCanFrame) -> String {
    let mut s = format!("{:08X}", f.can_id);
    for b in f.payload() {
        s.push_str(&format!(" {:02X}", b));
    }
    s.push_str("\r\n");
    s
}

/// candump `-L` (`(1631530412.123456) can0 09F80117#0102...`) or default
/// (`  can0  09F80117   [8]  01 02 ...`) formats.
pub fn parse_candump(line: &str) -> Option<RawCanFrame> {
    let l = line.trim();
    if l.starts_with('(') {
        let close = l.find(')')?;
        let ts: f64 = l[1..close].parse().ok()?;
        let rest: Vec<&str> = l[close + 1..].split_whitespace().collect();
        let fr = rest.get(1)?;
        let (id, data) = fr.split_once('#')?;
        let id = u32::from_str_radix(id, 16).ok()?;
        let bytes = crate::frame::parse_hex(data)?;
        return Some(RawCanFrame::new((ts * 1_000_000.0).round() as u64, id, &bytes));
    }
    let toks: Vec<&str> = l.split_whitespace().collect();
    if toks.len() >= 3 && toks[2].starts_with('[') {
        let id = u32::from_str_radix(toks[1], 16).ok()?;
        let n: usize = toks[2].trim_matches(|c| c == '[' || c == ']').parse().ok()?;
        let bytes = hexbytes(toks[3..].iter().take(n).copied())?;
        return Some(RawCanFrame::new(0, id, &bytes));
    }
    None
}

pub fn candump_line(unix_us: u64, iface: &str, f: &RawCanFrame) -> String {
    format!("({}.{:06}) {} {:08X}#{}\n", unix_us / 1_000_000, unix_us % 1_000_000, iface, f.can_id, hex(f.payload()).to_uppercase())
}

/// Parse an ISO-8601 UTC timestamp `YYYY-MM-DDTHH:MM:SS(.fff)Z` (or with a
/// space and/or without Z, or `YYYY-MM-DD-HH:MM:SS.fff`) to Unix µs.
pub fn parse_iso_time(s: &str) -> Option<u64> {
    let s = s.trim().trim_end_matches('Z');
    if s.len() < 19 {
        return None;
    }
    let y: i64 = s.get(0..4)?.parse().ok()?;
    let mo: i64 = s.get(5..7)?.parse().ok()?;
    let d: i64 = s.get(8..10)?.parse().ok()?;
    let rest = s.get(11..)?;
    let us = tod_us(rest)?;
    // days from civil (Hinnant)
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = if y2 >= 0 { y2 } else { y2 - 399 } / 400;
    let yoe = y2 - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some((days as u64) * 86_400_000_000 + us)
}

pub fn format_iso_time(unix_us: u64) -> String {
    let days = (unix_us / 86_400_000_000) as i64;
    let rem = unix_us % 86_400_000_000;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    let ms = rem / 1000;
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        y,
        m,
        d,
        ms / 3_600_000,
        (ms / 60_000) % 60,
        (ms / 1000) % 60,
        ms % 1000
    )
}

/// CANboat plain: `2017-03-13T01:00:00.146Z,2,127245,204,255,8,fc,f8,ff,7f,ff,7f,ff,ff`.
pub fn parse_canboat_plain(line: &str) -> Option<N2kMessage> {
    let p: Vec<&str> = line.trim().split(',').collect();
    if p.len() < 6 {
        return None;
    }
    let ts = parse_iso_time(p[0]).unwrap_or(0);
    let prio: u8 = p[1].trim().parse().ok()?;
    let pgn: u32 = p[2].trim().parse().ok()?;
    let src: u8 = p[3].trim().parse().ok()?;
    let dst: u8 = p[4].trim().parse().ok()?;
    let len: usize = p[5].trim().parse().ok()?;
    let data = hexbytes(p[6..].iter().map(|s| s.trim()).take(len))?;
    Some(N2kMessage { timestamp_us: ts, priority: prio, pgn, source: src, destination: dst, data })
}

pub fn canboat_plain_line(unix_us: u64, m: &N2kMessage) -> String {
    let mut s = format!("{},{},{},{},{},{}", format_iso_time(unix_us), m.priority, m.pgn, m.source, m.destination, m.data.len());
    for b in &m.data {
        s.push_str(&format!(",{:02x}", b));
    }
    s.push('\n');
    s
}

/// Actisense N2K ASCII: `A173321.107 23FF7 1F513 012F3070002F30709F`.
pub fn parse_actisense_ascii(line: &str) -> Option<N2kMessage> {
    let l = line.trim();
    let rest = l.strip_prefix('A')?;
    let mut it = rest.split_whitespace();
    let t = it.next()?;
    let ts = if t.len() >= 6 {
        let h: u64 = t[0..2].parse().ok()?;
        let m: u64 = t[2..4].parse().ok()?;
        let s: f64 = t[4..].parse().ok()?;
        h * 3_600_000_000 + m * 60_000_000 + (s * 1e6).round() as u64
    } else {
        0
    };
    let addr = it.next()?;
    if addr.len() != 5 {
        return None;
    }
    let src = u8::from_str_radix(&addr[0..2], 16).ok()?;
    let dst = u8::from_str_radix(&addr[2..4], 16).ok()?;
    let prio = u8::from_str_radix(&addr[4..5], 16).ok()?;
    let pgn = u32::from_str_radix(it.next()?, 16).ok()?;
    let data = crate::frame::parse_hex(it.next().unwrap_or(""))?;
    Some(N2kMessage { timestamp_us: ts, priority: prio, pgn, source: src, destination: dst, data })
}

/// Detect and parse any supported line format.
pub fn parse_any(line: &str) -> Option<LineItem> {
    let l = line.trim();
    if l.is_empty() || l.starts_with('#') {
        return None;
    }
    if let Some((_, f)) = parse_yd_raw(l) {
        return Some(LineItem::Frame(f));
    }
    if let Some(f) = parse_candump(l) {
        return Some(LineItem::Frame(f));
    }
    if l.starts_with('A') {
        if let Some(m) = parse_actisense_ascii(l) {
            return Some(LineItem::Message(m));
        }
    }
    parse_canboat_plain(l).map(LineItem::Message)
}

/// Convenience: CAN id for a message.
pub fn msg_can_id(m: &N2kMessage) -> u32 {
    CanId { priority: m.priority, pgn: m.pgn, destination: m.destination, source: m.source }.encode()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yd() {
        let (tx, f) = parse_yd_raw("17:33:21.107 R 19F51323 01 2F 30 70 00 2F 30 70").unwrap();
        assert!(!tx);
        assert_eq!(f.can_id, 0x19F51323);
        assert_eq!(f.payload(), &[1, 0x2F, 0x30, 0x70, 0, 0x2F, 0x30, 0x70]);
        assert_eq!(f.id().pgn, 0x1F513);
        assert_eq!(yd_raw_tx(&f), "19F51323 01 2F 30 70 00 2F 30 70\r\n");
    }

    #[test]
    fn candump() {
        let f = parse_candump("(1631530412.123456) can0 09F80117#0102030405060708").unwrap();
        assert_eq!(f.timestamp_us, 1_631_530_412_123_456);
        assert_eq!(f.dlc, 8);
        let g = parse_candump("  can0  18EA2380   [3]  14 F0 01").unwrap();
        assert_eq!(g.payload(), &[0x14, 0xF0, 0x01]);
        assert_eq!(candump_line(f.timestamp_us, "can0", &f), "(1631530412.123456) can0 09F80117#0102030405060708\n");
    }

    #[test]
    fn canboat_plain() {
        let m = parse_canboat_plain("2017-03-13T01:00:00.146Z,2,127245,204,255,8,fc,f8,ff,7f,ff,7f,ff,ff").unwrap();
        assert_eq!(m.pgn, 127245);
        assert_eq!(m.source, 204);
        assert_eq!(m.data.len(), 8);
        assert_eq!(format_iso_time(m.timestamp_us), "2017-03-13T01:00:00.146Z");
        assert_eq!(canboat_plain_line(m.timestamp_us, &m).trim(), "2017-03-13T01:00:00.146Z,2,127245,204,255,8,fc,f8,ff,7f,ff,7f,ff,ff");
    }

    #[test]
    fn actisense_ascii() {
        let m = parse_actisense_ascii("A173321.107 23FF7 1F513 012F3070002F30709F").unwrap();
        assert_eq!((m.source, m.destination, m.priority, m.pgn), (0x23, 0xFF, 7, 0x1F513));
        assert_eq!(m.data.len(), 9);
    }

    #[test]
    fn detect() {
        assert!(matches!(parse_any("17:33:21.107 R 19F51323 01"), Some(LineItem::Frame(_))));
        assert!(matches!(parse_any("2017-03-13T01:00:00.146Z,2,127245,204,255,1,fc"), Some(LineItem::Message(_))));
        assert!(parse_any("# comment").is_none());
    }
}

//! JSON shapes shared by REST and WebSocket.

use keelwire_core::canboat::PgnDb;
use keelwire_core::filter::{self, Lit};
use keelwire_core::frame::hex;
use keelwire_core::sniffer::PacketRecord;
use keelwire_core::Field;
use serde::Serialize;

#[derive(Serialize, Clone)]
pub struct PacketDto {
    pub seq: u64,
    pub ts: u64,
    pub prio: u8,
    pub pgn: u32,
    pub src: u8,
    pub dst: u8,
    pub len: usize,
    pub data: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub diff: Vec<i8>,
    pub frames: u8,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub tx: bool,
    pub name: String,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<Field>>,
}

pub struct Decoded<'a> {
    pub rec: &'a PacketRecord,
    pub dec: keelwire_core::canboat::Decoded,
}

impl filter::Context for Decoded<'_> {
    fn get(&self, id: &str) -> Option<Lit> {
        let r = self.rec;
        Some(match id {
            "pgn" => Lit::Num(r.pgn as f64),
            "src" | "source" => Lit::Num(r.source as f64),
            "dst" | "destination" => Lit::Num(r.destination as f64),
            "prio" | "priority" => Lit::Num(r.priority as f64),
            "len" => Lit::Num(r.data.len() as f64),
            "tx" => Lit::Num(r.tx as u8 as f64),
            "name" => Lit::Str(self.dec.description.clone()),
            "data" => Lit::Str(hex(&r.data)),
            "summary" => Lit::Str(self.dec.summary(32)),
            other => {
                let f = self.dec.fields.iter().find(|f| f.id == other || f.name.eq_ignore_ascii_case(other))?;
                if let Some(t) = &f.text {
                    if f.value.as_f64().is_none() {
                        return Some(Lit::Str(t.clone()));
                    }
                }
                match f.value.as_f64() {
                    Some(n) => Lit::Num(n),
                    None => Lit::Str(f.render()),
                }
            }
        })
    }
}

pub fn to_dto(db: &PgnDb, r: &PacketRecord, filter: Option<&filter::Expr>, with_fields: bool) -> Option<PacketDto> {
    let dec = db.decode(r.pgn, &r.data);
    if let Some(f) = filter {
        let ctx = Decoded { rec: r, dec };
        if !filter::eval(f, &ctx) {
            return None;
        }
        return Some(build(r, ctx.dec, with_fields));
    }
    Some(build(r, dec, with_fields))
}

fn build(r: &PacketRecord, dec: keelwire_core::canboat::Decoded, with_fields: bool) -> PacketDto {
    PacketDto {
        seq: r.seq,
        ts: r.timestamp_us,
        prio: r.priority,
        pgn: r.pgn,
        src: r.source,
        dst: r.destination,
        len: r.data.len(),
        data: hex(&r.data),
        diff: if r.diff.iter().any(|d| *d != 0) { r.diff.clone() } else { vec![] },
        frames: r.frames,
        tx: r.tx,
        summary: dec.summary(6),
        name: dec.description.clone(),
        fields: if with_fields { Some(dec.fields) } else { None },
    }
}

//! End of session: files the session in the boat's history folder and writes
//! the owner report (PDF), the technician report (PDF or HTML), CSV exports
//! and the session bundle (.zip).

#![allow(clippy::too_many_arguments)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use keelwire_core::canboat::PgnDb;
use keelwire_core::registry::Device;
use keelwire_core::traffic::TrafficSnapshot;

use crate::boatfile::BoatFile;
use crate::engine::{EndOptions, EndResult};
use crate::pdf::{hex_rgb, Font, Pdf, Rgb, LETTER};
use crate::session::{self as ss, csv_field, fmt_duration, fmt_lat_lon, friendly, local_fmt, utc_offset_label, DeviceSnap, FindingSnap, Session};

pub struct EndCtx {
    pub session: Session,
    pub boat_after: BoatFile,
    pub traffic: TrafficSnapshot,
    pub devices: Vec<Device>,
    pub open_dir: PathBuf,
    pub root: PathBuf,
    pub db: Arc<PgnDb>,
}

pub const OWNER_PDF: &str = "Owner report.pdf";
pub const TECH_PDF: &str = "Technician report.pdf";
pub const TECH_HTML: &str = "Technician report.html";

static GEIST: &[u8] = include_bytes!("../assets/fonts/Geist-Regular.ttf");
static GEIST_SEMI: &[u8] = include_bytes!("../assets/fonts/Geist-SemiBold.ttf");
static GEIST_BOLD: &[u8] = include_bytes!("../assets/fonts/Geist-Bold.ttf");
static GEIST_MONO: &[u8] = include_bytes!("../assets/fonts/GeistMono-Regular.ttf");

const R: usize = 0;
const SEMI: usize = 1;
const BOLD: usize = 2;
const MONO: usize = 3;

fn ink() -> Rgb {
    hex_rgb(0x0D1726)
}
fn muted() -> Rgb {
    hex_rgb(0x566173)
}
fn soft() -> Rgb {
    hex_rgb(0x4A5568)
}
fn green() -> Rgb {
    hex_rgb(0x13804B)
}
fn amber() -> Rgb {
    hex_rgb(0x9A5100)
}
fn red() -> Rgb {
    hex_rgb(0xC4302B)
}
fn line_c() -> Rgb {
    hex_rgb(0xD5DAE1)
}

// ------------------------------------------------------------------ finalize

fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to)?;
    std::fs::remove_file(from)
}

pub fn finalize(ctx: EndCtx, opts: &EndOptions) -> Result<EndResult, String> {
    let EndCtx { mut session, boat_after, traffic, devices, open_dir, root, db } = ctx;
    let vfolder = session.vessel.folder_name();
    let vdir = root.join(&vfolder);
    let base = local_fmt(session.started_ms, "%Y-%m-%d %H%M");
    let mut sname = base.clone();
    let mut k = 2;
    while vdir.join(&sname).exists() {
        sname = format!("{base} ({k})");
        k += 1;
    }
    let dir = vdir.join(&sname);
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;

    // Since the last visit: the previous session's end against this one's start.
    if let Some(id) = session.vessel.id() {
        if let Some(h) = ss::find_vessel(&root, &id) {
            session.visit_number = h.sessions.len() as u32 + 1;
            if let Some(p) = h.sessions.first() {
                session.previous = Some(ss::PrevVisit { dir: format!("{}/{}", h.folder, p.dir), started_ms: p.started_ms, ended_ms: p.ended_ms });
                if let Ok(t) = std::fs::read_to_string(root.join(&h.folder).join(&p.dir).join("session.json")) {
                    if let Ok(old) = serde_json::from_str::<Session>(&t) {
                        let b = if old.devices_end.is_empty() { &old.devices_start } else { &old.devices_end };
                        session.since_last = ss::diff_devices(b, &session.devices_start);
                    }
                }
            }
        } else {
            session.visit_number = 1;
        }
    } else {
        session.visit_number = session.visit_number.max(1);
    }

    // Move the open session's files.
    let mut captures = Vec::new();
    for f in &session.capture_files {
        let src = open_dir.join(f);
        if src.exists() {
            let dst = dir.join(f);
            if opts.strip_other_ais {
                let own = session.vessel.mmsi;
                ss::strip_other_ais(&src, &dst, own).map_err(|e| format!("capture: {e}"))?;
                let _ = std::fs::remove_file(&src);
            } else {
                move_file(&src, &dst).map_err(|e| format!("capture: {e}"))?;
            }
            captures.push(f.clone());
        }
    }
    let before = open_dir.join("boat-before.keelwire");
    if before.exists() {
        let _ = move_file(&before, &dir.join("boat-before.keelwire"));
    }
    let mut boat_after = boat_after;
    if opts.strip_other_ais {
        ss::strip_boat_ais(&mut boat_after, session.vessel.mmsi);
        let bp = dir.join("boat-before.keelwire");
        if let Some(mut b) = std::fs::read(&bp).ok().and_then(|t| serde_json::from_slice::<BoatFile>(&t).ok()) {
            ss::strip_boat_ais(&mut b, session.vessel.mmsi);
            let _ = std::fs::write(&bp, serde_json::to_vec_pretty(&b).unwrap_or_default());
        }
    }
    let _ = std::fs::write(dir.join("boat-after.keelwire"), serde_json::to_vec_pretty(&boat_after).unwrap_or_default());
    let _ = std::fs::write(dir.join("changes.csv"), changes_csv(&session));
    let _ = std::fs::write(dir.join("devices.csv"), devices_csv(&session));
    let _ = std::fs::write(dir.join("pgns.csv"), pgns_csv(&traffic, &devices, &db));
    if !session.notes.is_empty() {
        let notes: String = session.notes.iter().map(|n| format!("{}  {}\n", local_fmt(n.ts_ms, "%Y-%m-%d %H:%M"), n.text)).collect();
        let _ = std::fs::write(dir.join("notes.txt"), notes);
    }
    let file_info: Vec<(String, u64, Option<String>)> = captures.iter().map(|f| {
        let p = dir.join(f);
        (f.clone(), std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0), ss::sha256_file(&p))
    }).collect();
    let _ = std::fs::write(dir.join("session.json"), serde_json::to_vec_pretty(&session).unwrap_or_default());

    let mut outputs: Vec<String> = Vec::new();
    if opts.owner_pdf {
        std::fs::write(dir.join(OWNER_PDF), owner_pdf(&session)?).map_err(|e| e.to_string())?;
        outputs.push(OWNER_PDF.into());
    }
    match opts.technician.as_deref() {
        Some("html") => {
            std::fs::write(dir.join(TECH_HTML), tech_html(&session, &file_info)).map_err(|e| e.to_string())?;
            outputs.push(TECH_HTML.into());
        }
        Some("pdf") => {
            std::fs::write(dir.join(TECH_PDF), tech_pdf(&session, &file_info)?).map_err(|e| e.to_string())?;
            outputs.push(TECH_PDF.into());
        }
        _ => {}
    }
    if opts.bundle {
        let zname = format!("Keelwire session {sname}.zip");
        write_zip(&dir, &dir.join(&zname)).map_err(|e| format!("bundle: {e}"))?;
        outputs.push(zname);
    }
    let mut all: Vec<String> = std::fs::read_dir(&dir).map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect()).unwrap_or_default();
    all.sort();
    let summary = ss::summary_of(&session, &sname, all.clone());
    let _ = std::fs::write(dir.join("summary.json"), serde_json::to_vec_pretty(&summary).unwrap_or_default());
    let rec = ss::VesselRecord { id: session.vessel.id().unwrap_or_else(|| "unnamed".into()), vessel: session.vessel.clone(), folder: vfolder.clone() };
    let _ = std::fs::write(vdir.join("vessel.json"), serde_json::to_vec_pretty(&rec).unwrap_or_default());
    let _ = std::fs::remove_dir_all(&open_dir);
    // Reports and bundle first, then the rest.
    let mut files = outputs;
    for f in all {
        if !files.contains(&f) && f != "summary.json" {
            files.push(f);
        }
    }
    Ok(EndResult { dir: dir.display().to_string(), rel: format!("{vfolder}/{sname}"), files })
}

fn write_zip(dir: &Path, out: &Path) -> std::io::Result<()> {
    let f = std::fs::File::create(out)?;
    let mut z = zip::ZipWriter::new(f);
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut names: Vec<PathBuf> = std::fs::read_dir(dir)?.flatten().map(|e| e.path()).filter(|p| p.is_file() && p != out && p.extension().map(|x| x != "zip").unwrap_or(true)).collect();
    names.sort();
    for p in names {
        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        z.start_file(name, opts).map_err(std::io::Error::other)?;
        z.write_all(&std::fs::read(&p)?)?;
    }
    z.finish().map_err(std::io::Error::other)?;
    Ok(())
}

// ------------------------------------------------------------------ CSV

fn changes_csv(s: &Session) -> String {
    let mut o = String::from("time,device,serial,setting,before,after,method,result,error,kind\n");
    for c in &s.changes {
        let row = [
            local_fmt(c.ts_ms, "%Y-%m-%d %H:%M:%S"),
            c.device.clone(),
            c.serial.clone().unwrap_or_default(),
            c.setting.clone(),
            c.before.clone().unwrap_or_default(),
            c.after.clone().unwrap_or_default(),
            c.method.clone(),
            c.result.clone(),
            c.error.clone().unwrap_or_default(),
            c.kind.clone(),
        ];
        o += &row.iter().map(|x| csv_field(x)).collect::<Vec<_>>().join(",");
        o.push('\n');
    }
    o
}

fn devices_csv(s: &Session) -> String {
    let mut o = String::from("when,address,manufacturer,model,function,serial,software,device_instance,system_instance,installation1,installation2,len,online\n");
    for (when, list) in [("start", &s.devices_start), ("end", &s.devices_end)] {
        for d in list {
            let row = [
                when.to_string(),
                d.address.map(|a| a.to_string()).unwrap_or_default(),
                d.manufacturer.clone().unwrap_or_default(),
                d.model.clone().unwrap_or_default(),
                d.function.clone().unwrap_or_default(),
                d.serial.clone().unwrap_or_default(),
                d.software.clone().unwrap_or_default(),
                d.device_instance.map(|x| x.to_string()).unwrap_or_default(),
                d.system_instance.map(|x| x.to_string()).unwrap_or_default(),
                d.install1.clone().unwrap_or_default(),
                d.install2.clone().unwrap_or_default(),
                d.len.map(|x| x.to_string()).unwrap_or_default(),
                d.online.to_string(),
            ];
            o += &row.iter().map(|x| csv_field(x)).collect::<Vec<_>>().join(",");
            o.push('\n');
        }
    }
    o
}

fn pgns_csv(t: &TrafficSnapshot, devices: &[Device], db: &PgnDb) -> String {
    let v = serde_json::to_value(t).unwrap_or_default();
    let mut o = String::from("pgn,name,source,model,messages,frames\n");
    if let Some(rows) = v.get("pgns").and_then(|p| p.as_array()) {
        for r in rows {
            let pgn = r.get("pgn").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
            let src = r.get("source").and_then(|x| x.as_u64()).unwrap_or(0) as u8;
            let model = devices.iter().find(|d| d.address == Some(src)).and_then(|d| d.model.clone()).unwrap_or_default();
            let row = [pgn.to_string(), db.name(pgn).unwrap_or("").to_string(), src.to_string(), model, r.get("messages").map(|x| x.to_string()).unwrap_or_default(), r.get("frames").map(|x| x.to_string()).unwrap_or_default()];
            o += &row.iter().map(|x| csv_field(x)).collect::<Vec<_>>().join(",");
            o.push('\n');
        }
    }
    o
}

// ------------------------------------------------------------------ plain language

fn dev_for_key<'a>(s: &'a Session, key: &str) -> Option<&'a DeviceSnap> {
    s.devices_end.iter().chain(&s.devices_start).find(|d| d.key == key)
}

/// A finding in the owner's words.
pub fn plain_finding(s: &Session, f: &FindingSnap, fixed: bool) -> String {
    let kind = f.sig.split('|').next().unwrap_or("");
    let names: Vec<String> = f.devices.iter().filter_map(|k| dev_for_key(s, k)).map(friendly).collect();
    match kind {
        "instance_conflict" => {
            let what = match f.pgn {
                Some(127505) => "The tanks were reporting as the same tank, so displays could show the wrong level.".to_string(),
                Some(127508) | Some(127506) => "Two battery monitors were reporting as the same battery, so displays could mix up their readings.".to_string(),
                Some(127488) | Some(127489) => "Two engines were reporting as the same engine, so displays could mix up their readings.".to_string(),
                Some(130312) | Some(130316) => "Two temperature sensors were reporting as the same sensor.".to_string(),
                _ => format!("{} devices were reporting the same kind of data under the same number, so displays could mix up their readings.", names.len().max(2)),
            };
            if fixed {
                format!("{what} They now report separately.")
            } else {
                format!("{what} They need different instance numbers.")
            }
        }
        "duplicate_device_instance" => {
            if fixed {
                "Two devices of the same kind had the same device number, which can confuse displays. Each now has its own.".into()
            } else {
                "Two devices of the same kind have the same device number, which can confuse displays.".into()
            }
        }
        "no_product_info" => format!("{} does not identify itself on the network; it may need a software update or a check of its connection.", names.first().cloned().unwrap_or_else(|| "A device".into())),
        "unclaimed_source" => "A device is sending data without identifying itself on the network.".into(),
        _ => f.message.clone(),
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

fn len_total(list: &[DeviceSnap]) -> (u32, usize) {
    let total: u32 = list.iter().filter_map(|d| d.len).map(|l| l as u32).sum();
    let missing = list.iter().filter(|d| d.len.is_none() && d.model.is_some()).count();
    (total, missing)
}

fn time_range(s: &Session) -> String {
    let end = s.ended_ms.unwrap_or(s.started_ms);
    format!("{} – {} ({})", local_fmt(s.started_ms, "%H:%M"), local_fmt(end, "%H:%M"), utc_offset_label(s.started_ms))
}

fn offline_at_end(s: &Session) -> Vec<&DeviceSnap> {
    s.devices_end.iter().filter(|d| !d.online && d.model.is_some()).collect()
}

// ------------------------------------------------------------------ PDF layout

struct Doc {
    pdf: Pdf,
    page: usize,
    y: f32,
    left: f32,
    right: f32,
    top: f32,
    bottom: f32,
    footer_left: String,
}

impl Doc {
    fn new(title: &str, margin: f32, footer_left: String) -> Result<Doc, String> {
        let fonts = vec![Font::load(GEIST)?, Font::load(GEIST_SEMI)?, Font::load(GEIST_BOLD)?, Font::load(GEIST_MONO)?];
        let mut pdf = Pdf::new(LETTER, fonts, title);
        let page = pdf.add_page();
        Ok(Doc { pdf, page, y: margin, left: margin, right: LETTER.0 - margin, top: margin, bottom: LETTER.1 - margin - 18.0, footer_left })
    }

    fn width(&self) -> f32 {
        self.right - self.left
    }

    fn new_page(&mut self) {
        self.page = self.pdf.add_page();
        self.y = self.top;
    }

    fn ensure(&mut self, h: f32) {
        if self.y + h > self.bottom {
            self.new_page();
        }
    }

    fn wrap(&self, font: usize, size: f32, text: &str, width: f32) -> Vec<String> {
        let f = &self.pdf.fonts[font];
        let mut lines = Vec::new();
        for para in text.split('\n') {
            let mut cur = String::new();
            for word in para.split(' ') {
                let cand = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
                if f.width(size, &cand) <= width || cur.is_empty() {
                    // Break a single over-long word by characters.
                    if cur.is_empty() && f.width(size, &cand) > width {
                        let mut part = String::new();
                        for ch in word.chars() {
                            let c2 = format!("{part}{ch}");
                            if f.width(size, &c2) > width && !part.is_empty() {
                                lines.push(part);
                                part = ch.to_string();
                            } else {
                                part = c2;
                            }
                        }
                        cur = part;
                    } else {
                        cur = cand;
                    }
                } else {
                    lines.push(cur);
                    cur = word.to_string();
                }
            }
            lines.push(cur);
        }
        lines
    }

    fn text_at(&mut self, x: f32, y: f32, font: usize, size: f32, color: Rgb, s: &str) {
        let p = self.page;
        self.pdf.text(p, x, y, font, size, color, s);
    }

    fn text_right(&mut self, x_right: f32, y: f32, font: usize, size: f32, color: Rgb, s: &str) {
        let w = self.pdf.fonts[font].width(size, s);
        self.text_at(x_right - w, y, font, size, color, s);
    }

    /// Paragraph at the cursor; returns its height.
    fn para(&mut self, text: &str, font: usize, size: f32, color: Rgb, x: f32, width: f32, lh: f32) -> f32 {
        let lines = self.wrap(font, size, text, width);
        let mut h = 0.0;
        for l in lines {
            self.ensure(lh);
            let base = self.y + size;
            self.text_at(x, base, font, size, color, &l);
            self.y += lh;
            h += lh;
        }
        h
    }

    /// Mixed-font paragraph: runs of (text, font, color).
    fn runs(&mut self, runs: &[(String, usize, Rgb)], size: f32, x: f32, width: f32, lh: f32) {
        // Split into words keeping their style.
        let mut words: Vec<(String, usize, Rgb)> = Vec::new();
        for (t, f, c) in runs {
            for (i, w) in t.split(' ').enumerate() {
                if i > 0 {
                    words.push((" ".into(), *f, *c));
                }
                if !w.is_empty() {
                    words.push((w.to_string(), *f, *c));
                }
            }
        }
        let mut cx = x;
        self.ensure(lh);
        for (w, f, c) in words {
            let ww = self.pdf.fonts[f].width(size, &w);
            if w == " " {
                if cx > x {
                    cx += ww;
                }
                continue;
            }
            if cx + ww > x + width && cx > x {
                self.y += lh;
                self.ensure(lh);
                cx = x;
            }
            let base = self.y + size;
            self.text_at(cx, base, f, size, c, &w);
            cx += ww;
        }
        self.y += lh;
    }

    fn heading(&mut self, text: &str, size: f32) {
        self.ensure(size * 2.6);
        self.y += size * 0.5;
        let b = self.y + size;
        self.text_at(self.left, b, BOLD, size, ink(), text);
        self.y += size * 1.6;
    }

    fn rule(&mut self, color: Rgb, w: f32) {
        let p = self.page;
        self.pdf.line(p, self.left, self.y, self.right, self.y, color, w);
    }

    /// A tinted box around a paragraph with a leading mark.
    fn callout(&mut self, mark: &str, mark_color: Rgb, fill: Rgb, border: Rgb, text_runs: &[(String, usize, Rgb)], size: f32) {
        let pad = 9.0;
        let indent = 18.0;
        let width = self.width() - 2.0 * pad - indent;
        // Measure.
        let mut words_h = 0.0;
        {
            let plain: String = text_runs.iter().map(|r| r.0.clone()).collect();
            words_h += self.wrap(SEMI, size, &plain, width).len() as f32 * size * 1.45;
        }
        let h = words_h + 2.0 * pad - size * 0.25;
        self.ensure(h + 6.0);
        let (p, y0) = (self.page, self.y);
        self.pdf.round_rect(p, self.left, y0, self.width(), h, 7.0, Some(fill), Some((border, 0.8)));
        self.text_at(self.left + pad, y0 + pad + size, BOLD, size, mark_color, mark);
        self.y = y0 + pad;
        self.runs(text_runs, size, self.left + pad + indent, width, size * 1.45);
        self.y = y0 + h + 7.0;
    }

    /// Table with wrapped cells. `cols` = (header, width fraction, font).
    fn table(&mut self, cols: &[(&str, f32, usize)], rows: &[Vec<(String, Option<Rgb>)>], size: f32) {
        let w = self.width();
        let widths: Vec<f32> = cols.iter().map(|c| c.1 * w).collect();
        let pad = 4.0;
        let lh = size * 1.35;
        let header = |d: &mut Doc| {
            d.ensure(lh + 2.0 * pad + lh);
            let mut x = d.left;
            for (i, c) in cols.iter().enumerate() {
                let yb = d.y + pad + size;
                d.text_at(x + if i == 0 { 0.0 } else { pad }, yb, SEMI, size, soft(), c.0);
                x += widths[i];
            }
            d.y += lh + 2.0 * pad;
            d.rule(ink(), 0.8);
        };
        let has_header = cols.iter().any(|c| !c.0.is_empty());
        if has_header {
            header(self);
        } else {
            self.rule(line_c(), 0.5);
        }
        for row in rows {
            let cells: Vec<Vec<String>> = row.iter().enumerate().map(|(i, (t, _))| self.wrap(cols[i].2, size, t, widths[i] - 2.0 * pad)).collect();
            let n = cells.iter().map(|c| c.len()).max().unwrap_or(1);
            let h = n as f32 * lh + 2.0 * pad;
            if self.y + h > self.bottom {
                self.new_page();
                if has_header {
                    header(self);
                }
            }
            let mut x = self.left;
            for (i, lines) in cells.iter().enumerate() {
                let color = row[i].1.unwrap_or(ink());
                for (k, l) in lines.iter().enumerate() {
                    let yb = self.y + pad + size + k as f32 * lh;
                    self.text_at(x + if i == 0 { 0.0 } else { pad }, yb, cols[i].2, size, color, l);
                }
                x += widths[i];
            }
            self.y += h;
            self.rule(line_c(), 0.5);
        }
        self.y += 6.0;
    }

    /// Two-column label/value list.
    fn kv(&mut self, items: &[(String, String)], label_w: f32, size: f32) {
        let lh = size * 1.5;
        for (k, v) in items {
            let lines = self.wrap(R, size, v, self.width() - label_w);
            let h = lines.len() as f32 * lh + 4.0;
            self.ensure(h);
            self.rule(line_c(), 0.5);
            let y0 = self.y + 2.0;
            self.text_at(self.left, y0 + size, R, size, soft(), k);
            for (i, l) in lines.iter().enumerate() {
                self.text_at(self.left + label_w, y0 + size + i as f32 * lh, R, size, ink(), l);
            }
            self.y += h;
        }
        self.rule(line_c(), 0.5);
        self.y += 6.0;
    }

    fn finish(mut self, footer_right: &dyn Fn(usize, usize) -> String) -> Vec<u8> {
        let n = self.pdf.page_count();
        for p in 0..n {
            let y = LETTER.1 - self.top + 8.0;
            self.pdf.line(p, self.left, y - 12.0, self.right, y - 12.0, line_c(), 0.5);
            let fl = self.footer_left.clone();
            self.pdf.text(p, self.left, y, R, 8.5, hex_rgb(0x6B7686), &fl);
            let fr = footer_right(p + 1, n);
            let w = self.pdf.fonts[R].width(8.5, &fr);
            let right = self.right;
            self.pdf.text(p, right - w, y, R, 8.5, hex_rgb(0x6B7686), &fr);
        }
        self.pdf.finish()
    }
}

fn header_block(d: &mut Doc, s: &Session, kicker: &str, name_size: f32, right: &[String], sub: &[String]) {
    let top = d.y;
    d.text_at(d.left, top + 9.0, SEMI, 8.5, muted(), &kicker.to_uppercase());
    let name = s.vessel.display_name();
    d.text_at(d.left, top + 14.0 + name_size, BOLD, name_size, ink(), &name);
    let mut y = top + 14.0 + name_size + 6.0;
    for l in sub {
        y += 13.0;
        d.text_at(d.left, y, R, 9.5, soft(), l);
    }
    let mut ry = top + 9.0;
    for (i, l) in right.iter().enumerate() {
        let (f, c) = if i == 0 { (SEMI, ink()) } else { (R, soft()) };
        d.text_right(d.right, ry, f, 9.5, c, l);
        ry += 13.5;
    }
    d.y = y.max(ry) + 10.0;
    d.rule(ink(), 1.6);
    d.y += 14.0;
}

fn id_line(s: &Session) -> Vec<String> {
    let mut parts = Vec::new();
    if let Some(m) = s.vessel.mmsi {
        parts.push(format!("MMSI {m}"));
    }
    if let Some(c) = &s.vessel.callsign {
        parts.push(format!("call sign {c}"));
    }
    if let Some(h) = &s.vessel.hin {
        parts.push(format!("HIN {h}"));
    }
    let mut out = Vec::new();
    if !parts.is_empty() {
        out.push(parts.join(" · "));
    }
    if let Some(p) = &s.position {
        out.push(format!("Position during service: {}", fmt_lat_lon(p.lat, p.lon)));
    }
    out
}

// ------------------------------------------------------------------ owner report

pub fn owner_pdf(s: &Session) -> Result<Vec<u8>, String> {
    let mut d = Doc::new(&format!("{} – network service report", s.vessel.display_name()), 50.0, format!("Prepared with Keelwire {} · technical details in the technician report", s.app_version))?;
    let mut right = vec![local_fmt(s.started_ms, "%A %-d %B %Y"), time_range(s)];
    right.push(if s.visit_number > 1 { format!("Visit {} for this boat", s.visit_number) } else { "First visit for this boat".into() });
    if let Some(t) = &s.technician {
        right.push(format!("Technician: {t}"));
    }
    header_block(&mut d, s, "Network service report", 26.0, &right, &id_line(s));

    let devs = &s.devices_end;
    let working = devs.iter().filter(|x| x.online).count();
    let fixed = s.fixed();
    let open = s.open();
    let offline = offline_at_end(s);
    let changes: Vec<&ss::Change> = s.ok_changes().collect();
    let confirmed = changes.iter().filter(|c| c.result == "confirmed").count();
    let attention = open.len() + offline.len();

    // In short
    let mut runs: Vec<(String, usize, Rgb)> = vec![("Your boat’s instrument network has ".into(), R, ink()), (format!("{}; {} working", plural(devs.len(), "device", "devices"), working), SEMI, ink()), (" at the end of the visit. ".into(), R, ink())];
    if !fixed.is_empty() {
        runs.push(("We ".into(), R, ink()));
        runs.push((format!("fixed {}", plural(fixed.len(), "problem", "problems")), SEMI, ink()));
        runs.push((if changes.is_empty() { ". ".into() } else { " and ".into() }, R, ink()));
    }
    if !changes.is_empty() {
        if fixed.is_empty() {
            runs.push(("We ".into(), R, ink()));
        }
        runs.push((format!("made {}", plural(changes.len(), "change", "changes")), SEMI, ink()));
        runs.push((
            if confirmed == changes.len() { if changes.len() == 1 { ", confirmed by the device. ".into() } else { ", each confirmed by the device. ".into() } } else { format!(", {confirmed} confirmed by the device. ") },
            R,
            ink(),
        ));
    }
    if fixed.is_empty() && changes.is_empty() {
        runs.push(("We checked the network and made no changes. ".into(), R, ink()));
    }
    if attention > 0 {
        runs.push((format!("{} still {}.", plural(attention, "item", "items"), if attention == 1 { "needs attention" } else { "need attention" }), SEMI, ink()));
    } else {
        runs.push(("Nothing else needs attention.".into(), R, ink()));
    }
    {
        let pad = 12.0;
        let width = d.width() - 2.0 * pad;
        let plain: String = runs.iter().map(|r| r.0.clone()).collect();
        let h = d.wrap(SEMI, 11.5, &plain, width).len() as f32 * 16.5 + 2.0 * pad + 18.0;
        let (p, y0) = (d.page, d.y);
        d.pdf.round_rect(p, d.left, y0, d.width(), h, 9.0, Some(hex_rgb(0xF6F7F9)), None);
        d.text_at(d.left + pad, y0 + pad + 12.0, BOLD, 13.0, ink(), "In short");
        d.y = y0 + pad + 20.0;
        d.runs(&runs, 11.5, d.left + pad, width, 16.5);
        d.y = y0 + h + 8.0;
    }

    if !fixed.is_empty() {
        d.heading("What we fixed", 13.0);
        for f in &fixed {
            let t = plain_finding(s, f, true);
            d.callout("+", green(), hex_rgb(0xF4FAF6), hex_rgb(0xCDE8D8), &[(t, R, ink())], 10.5);
        }
    }
    let mut fixed_plain: Vec<String> = Vec::new();
    for p in changes.iter().filter_map(|c| c.plain.clone()) {
        if !fixed_plain.contains(&p) {
            fixed_plain.push(p);
        }
    }
    if !fixed_plain.is_empty() {
        d.heading("Changes we made", 13.0);
        for p in fixed_plain {
            d.ensure(16.0);
            let y = d.y;
            d.text_at(d.left + 4.0, y + 10.5, BOLD, 10.5, muted(), "·");
            let w = d.width() - 16.0;
            let x = d.left + 16.0;
            d.para(&p, R, 10.5, ink(), x, w, 15.0);
            d.y += 2.0;
        }
    }
    if attention > 0 {
        d.heading("Still needs attention", 13.0);
        for f in &open {
            let mut t = plain_finding(s, f, false);
            if let Some(sg) = &f.suggestion {
                t = format!("{t} {sg}");
            }
            d.callout("!", amber(), hex_rgb(0xFFF9F1), hex_rgb(0xF0D9B8), &[(t, R, ink())], 10.5);
        }
        for x in &offline {
            let t = format!("{} stopped responding during the visit. Check its cable and connector.", friendly(x));
            d.callout("!", amber(), hex_rgb(0xFFF9F1), hex_rgb(0xF0D9B8), &[(t, R, ink())], 10.5);
        }
    }
    if !s.notes.is_empty() {
        d.heading("Notes from the technician", 13.0);
        for n in &s.notes {
            d.para(&n.text, R, 10.5, ink(), d.left, d.width(), 15.0);
            d.y += 4.0;
        }
    }
    if !s.since_last.is_empty() {
        d.heading("Since the last visit", 13.0);
        if let Some(p) = &s.previous {
            d.para(&format!("Compared with {}.", local_fmt(p.started_ms, "%-d %B %Y")), R, 9.5, soft(), d.left, d.width(), 13.0);
        }
        for i in &s.since_last {
            d.ensure(16.0);
            let y = d.y;
            d.text_at(d.left + 4.0, y + 10.5, BOLD, 10.5, muted(), "·");
            let (x, w) = (d.left + 16.0, d.width() - 16.0);
            d.para(&i.plain, R, 10.5, ink(), x, w, 15.0);
            d.y += 2.0;
        }
        d.y += 4.0;
    }

    d.heading("Network health", 13.0);
    let mut rows: Vec<Vec<(String, Option<Rgb>)>> = Vec::new();
    if let Some(avg) = s.load.avg() {
        let (verdict, c) = if avg < 30.0 { ("Healthy (well under 30 %)", green()) } else if avg < 50.0 { ("Busy", amber()) } else { ("Very busy: check with your technician", red()) };
        rows.push(vec![("Network traffic".into(), Some(soft())), (format!("{avg:.1} % of capacity"), None), (verdict.into(), Some(c))]);
    }
    let (len, _) = len_total(devs);
    if len > 0 {
        rows.push(vec![("Power the devices may draw".into(), Some(soft())), (format!("up to {:.2} A", len as f64 * 0.05), None), ("Devices’ stated maximum".into(), Some(soft()))]);
    }
    rows.push(vec![
        ("Devices working".into(), Some(soft())),
        (format!("{working} of {}", devs.len()), None),
        (if working == devs.len() { "All working".into() } else { "See above".into() }, Some(if working == devs.len() { green() } else { amber() })),
    ]);
    d.table(&[("", 0.36, R), ("", 0.28, R), ("", 0.36, R)], &rows, 10.0);

    let names: Vec<String> = devs.iter().filter(|x| x.model.is_some() || x.function.is_some()).map(friendly).collect();
    let half = names.len().div_ceil(2).max(1);
    // Keep the two columns on one page.
    d.ensure(13.0 * 2.6 + half as f32 * 14.0);
    d.heading("Devices on your network", 13.0);
    let colw = d.width() / 2.0 - 10.0;
    let y0 = d.y;
    for (ci, chunk) in names.chunks(half).enumerate() {
        let x = d.left + ci as f32 * (colw + 20.0);
        for (k, n) in chunk.iter().enumerate() {
            let line = d.wrap(R, 10.0, n, colw).into_iter().next().unwrap_or_default();
            d.text_at(x, y0 + 10.0 + k as f32 * 14.0, R, 10.0, soft(), &line);
        }
    }
    d.y = y0 + half as f32 * 14.0;
    Ok(d.finish(&|p, n| format!("Page {p} of {n}")))
}

// ------------------------------------------------------------------ technician report

fn result_color(r: &str) -> Rgb {
    match r {
        "confirmed" => green(),
        "failed" => red(),
        _ => amber(),
    }
}

fn result_text(c: &ss::Change) -> String {
    match c.result.as_str() {
        "confirmed" => "Confirmed".into(),
        "sent" => "Accepted, not read back".into(),
        _ => format!("Failed: {}", c.error.clone().unwrap_or_default()),
    }
}

fn header_facts(s: &Session) -> Vec<(String, String)> {
    let mut v = vec![];
    if let Some(m) = s.vessel.mmsi {
        v.push(("MMSI".into(), m.to_string()));
    }
    if let Some(c) = &s.vessel.callsign {
        v.push(("Call sign".into(), c.clone()));
    }
    v.push(("HIN".into(), s.vessel.hin.clone().unwrap_or_else(|| "—".into())));
    v.push((
        "Identity".into(),
        match s.vessel.source.as_str() {
            "ais" => format!("AIS Class {}{}", s.vessel.ais_class.clone().unwrap_or_else(|| "?".into()), if s.vessel.confirmed { ", confirmed" } else { "" }),
            "manual" => "Typed by the technician".into(),
            _ => "Not identified".into(),
        },
    ));
    if let Some(p) = &s.position {
        v.push(("Position".into(), format!("{} ({})", fmt_lat_lon(p.lat, p.lon), p.source)));
    }
    v.push(("Technician".into(), s.technician.clone().unwrap_or_else(|| "—".into())));
    v.push(("Gateway".into(), s.gateway.clone().unwrap_or_else(|| "—".into())));
    v.push(("Session".into(), s.id.clone()));
    v.push(("Keelwire".into(), s.app_version.clone()));
    v
}

fn bus_facts(s: &Session, files: &[(String, u64, Option<String>)]) -> Vec<(String, String)> {
    let mut v = Vec::new();
    if let Some(avg) = s.load.avg() {
        v.push(("Load".into(), format!("{avg:.1} % average · {:.1} % peak{}", s.load.peak, if s.load.peak_ms > 0 { format!(" ({})", local_fmt(s.load.peak_ms, "%H:%M")) } else { String::new() })));
    }
    let bytes: u64 = files.iter().map(|f| f.1).sum();
    v.push(("Messages / frames".into(), format!("{} / {} · {:.1} MB recorded", group(s.messages), group(s.frames), bytes as f64 / 1e6)));
    v.push(("Bus errors".into(), s.bus_errors.map(|e| e.to_string()).unwrap_or_else(|| "Not reported by this gateway".into())));
    v.push(("Incomplete multi-frame".into(), s.incomplete.to_string()));
    let (len, missing) = len_total(&s.devices_end);
    v.push(("Declared load (LEN)".into(), format!("{len} LEN, {:.2} A max{}", len as f64 * 0.05, if missing > 0 { format!(" ({} did not report LEN)", plural(missing, "device", "devices")) } else { String::new() })));
    let active = s.active_ms(s.ended_ms.unwrap_or(s.started_ms));
    v.push(("Recording time".into(), fmt_duration(active)));
    if !s.pauses.is_empty() {
        let p: Vec<String> = s.pauses.iter().map(|p| format!("{}–{} {}", local_fmt(p.from_ms, "%H:%M:%S"), p.to_ms.map(|t| local_fmt(t, "%H:%M:%S")).unwrap_or_else(|| "end".into()), match p.reason.as_str() { "disconnected" => "gateway disconnected", "closed" => "Keelwire closed", "background" => "app in the background", _ => "paused by technician" })).collect();
        v.push(("Paused".into(), p.join("; ")));
    }
    v.push(("Other vessels’ AIS".into(), format!("{} heard, omitted from this report", s.other_ships)));
    v
}

fn group(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub fn tech_pdf(s: &Session, files: &[(String, u64, Option<String>)]) -> Result<Vec<u8>, String> {
    let mut d = Doc::new(&format!("{} – technician report", s.vessel.display_name()), 42.0, format!("Keelwire {} · other vessels’ AIS omitted from this report", s.app_version))?;
    let end = s.ended_ms.unwrap_or(s.started_ms);
    let right = vec![local_fmt(s.started_ms, "%a %-d %b %Y"), format!("{} – {} {}", local_fmt(s.started_ms, "%H:%M:%S"), local_fmt(end, "%H:%M:%S"), utc_offset_label(s.started_ms)), fmt_duration(end.saturating_sub(s.started_ms))];
    header_block(&mut d, s, "Technician report", 20.0, &right, &[]);
    // Facts in 2 columns.
    let facts = header_facts(s);
    let half = facts.len().div_ceil(2);
    let colw = d.width() / 2.0;
    let y0 = d.y;
    let mut ymax = y0;
    for (ci, chunk) in facts.chunks(half).enumerate() {
        let mut y = y0;
        for (k, v) in chunk {
            let x = d.left + ci as f32 * colw;
            d.text_at(x, y + 9.0, R, 8.5, muted(), k);
            let lines = d.wrap(R, 9.0, v, colw - 80.0);
            for (i, l) in lines.iter().enumerate() {
                d.text_at(x + 72.0, y + 9.0 + i as f32 * 12.0, R, 9.0, ink(), l);
            }
            y += 12.0 * lines.len() as f32 + 2.0;
        }
        ymax = ymax.max(y);
    }
    d.y = ymax + 8.0;

    let settings: Vec<&ss::Change> = s.changes.iter().filter(|c| c.kind == "setting").collect();
    let ok = settings.iter().filter(|c| c.result != "failed").count();
    let conf = settings.iter().filter(|c| c.result == "confirmed").count();
    d.heading(&format!("Changes sent ({}{})", ok, if ok > 0 && conf == ok { ", all confirmed" } else if ok > 0 { &"" } else { "" }), 11.5);
    if settings.is_empty() {
        d.para("No settings were changed.", R, 9.5, soft(), d.left, d.width(), 13.0);
    } else {
        let rows: Vec<Vec<(String, Option<Rgb>)>> = settings
            .iter()
            .map(|c| {
                vec![
                    (local_fmt(c.ts_ms, "%H:%M:%S"), None),
                    (format!("{}{}", c.device, c.serial.as_ref().map(|x| format!("\ns/n {x}")).unwrap_or_default()), None),
                    (c.setting.clone(), None),
                    (c.before.clone().unwrap_or_else(|| "—".into()), None),
                    (c.after.clone().unwrap_or_else(|| "—".into()), None),
                    (c.method.clone(), Some(soft())),
                    (result_text(c), Some(result_color(&c.result))),
                ]
            })
            .collect();
        d.table(&[("Time", 0.105, MONO), ("Device", 0.15, R), ("Setting", 0.21, R), ("Before", 0.12, MONO), ("After", 0.12, MONO), ("How", 0.15, R), ("Result", 0.145, R)], &rows, 8.5);
    }
    let other: Vec<&ss::Change> = s.changes.iter().filter(|c| c.kind != "setting").collect();
    if !other.is_empty() {
        d.heading("Control and expert commands", 11.5);
        let rows: Vec<Vec<(String, Option<Rgb>)>> = other.iter().map(|c| vec![(local_fmt(c.ts_ms, "%H:%M:%S"), None), (c.device.clone(), None), (c.setting.clone(), None), (c.after.clone().unwrap_or_default(), None), (result_text(c), Some(result_color(&c.result)))]).collect();
        d.table(&[("Time", 0.11, MONO), ("Target", 0.17, R), ("Command", 0.34, R), ("Value", 0.18, MONO), ("Result", 0.2, R)], &rows, 8.5);
    }

    d.heading("Findings", 11.5);
    let fixed = s.fixed();
    let open = s.open();
    if fixed.is_empty() && open.is_empty() && offline_at_end(s).is_empty() {
        d.para("No problems found.", R, 9.5, soft(), d.left, d.width(), 13.0);
    }
    for f in fixed {
        d.runs(&[("Fixed: ".into(), SEMI, green()), (format!("{} (found {}, last seen {}).", f.message, local_fmt(f.first_ms, "%H:%M"), local_fmt(f.last_ms, "%H:%M")), R, ink())], 9.5, d.left, d.width(), 13.5);
    }
    for f in open {
        d.runs(&[("Open: ".into(), SEMI, amber()), (format!("{}{}", f.message, f.suggestion.as_ref().map(|x| format!(" — {x}")).unwrap_or_default()), R, ink())], 9.5, d.left, d.width(), 13.5);
    }
    for x in offline_at_end(s) {
        d.runs(&[("Open: ".into(), SEMI, amber()), (format!("{} not responding at the end of the session{}.", x.label(), x.serial.as_ref().map(|s| format!(" (s/n {s})")).unwrap_or_default()), R, ink())], 9.5, d.left, d.width(), 13.5);
    }
    if let Some(p) = &s.previous {
        d.heading(&format!("Since last visit ({})", local_fmt(p.started_ms, "%-d %b %Y")), 11.5);
        if s.since_last.is_empty() {
            d.para("No changes to devices, software or settings.", R, 9.5, soft(), d.left, d.width(), 13.0);
        }
        for i in &s.since_last {
            let k = match i.kind.as_str() {
                "added" => "Added: ",
                "gone" => "Gone: ",
                "software" => "Software: ",
                _ => "Setting: ",
            };
            d.runs(&[(k.into(), SEMI, ink()), (i.text.clone(), R, ink())], 9.5, d.left, d.width(), 13.5);
        }
    }
    d.heading("Bus over the session", 11.5);
    d.kv(&bus_facts(s, files), 150.0, 9.0);
    if !s.notes.is_empty() {
        d.heading("Notes", 11.5);
        for n in &s.notes {
            d.runs(&[(format!("{} · ", local_fmt(n.ts_ms, "%H:%M")), MONO, muted()), (n.text.clone(), R, ink())], 9.5, d.left, d.width(), 13.5);
            d.y += 2.0;
        }
    }
    d.heading("Timeline", 11.5);
    let rows: Vec<Vec<(String, Option<Rgb>)>> = timeline(s).into_iter().map(|(t, txt, c)| vec![(local_fmt(t, "%H:%M:%S"), Some(muted())), (txt, c)]).collect();
    d.table(&[("Time", 0.12, MONO), ("Event", 0.88, R)], &rows, 8.5);
    d.heading("Files in this session", 11.5);
    for (f, size, sha) in files {
        d.para(&format!("{f} · {:.1} MB · sha256 {}", *size as f64 / 1e6, sha.clone().unwrap_or_default()), MONO, 8.0, soft(), d.left, d.width(), 11.5);
    }
    d.para("boat-before.keelwire · boat-after.keelwire · changes.csv · devices.csv · pgns.csv · session.json", MONO, 8.0, soft(), d.left, d.width(), 11.5);

    // Devices at the end of the session.
    d.new_page();
    d.heading("Devices at the end of the session", 11.5);
    let start_by: std::collections::BTreeMap<String, &DeviceSnap> = s.devices_start.iter().map(|x| (x.ident(), x)).collect();
    let rows: Vec<Vec<(String, Option<Rgb>)>> = s
        .devices_end
        .iter()
        .map(|x| {
            let changed = start_by.get(&x.ident()).map(|b| *b != x && (b.device_instance != x.device_instance || b.install1 != x.install1 || b.install2 != x.install2 || b.instances != x.instances)).unwrap_or(false);
            let inst: Vec<String> = x.instances.iter().take(3).map(|(p, k)| format!("{p}: {}", k.join("; "))).collect();
            vec![
                (x.address.map(|a| a.to_string()).unwrap_or_default(), None),
                (format!("{}{}", x.model.clone().unwrap_or_else(|| "—".into()), if changed { " *" } else { "" }), None),
                (x.manufacturer.clone().unwrap_or_default(), Some(soft())),
                (x.serial.clone().unwrap_or_default(), None),
                (x.software.clone().unwrap_or_default(), None),
                (x.device_instance.map(|i| i.to_string()).unwrap_or_default(), None),
                (inst.join("\n"), Some(soft())),
                (x.install1.clone().unwrap_or_default(), Some(soft())),
                (x.len.map(|l| l.to_string()).unwrap_or_default(), None),
                (if x.online { "online".into() } else { "offline".into() }, Some(if x.online { green() } else { amber() })),
            ]
        })
        .collect();
    d.table(
        &[("Addr", 0.05, MONO), ("Model", 0.11, R), ("Maker", 0.12, R), ("Serial", 0.1, MONO), ("Software", 0.09, MONO), ("Inst", 0.04, MONO), ("Data instances", 0.22, R), ("Installation", 0.13, R), ("LEN", 0.04, MONO), ("Status", 0.1, R)],
        &rows,
        7.5,
    );
    d.para("* settings changed during this session", R, 8.0, muted(), d.left, d.width(), 11.0);
    Ok(d.finish(&|p, n| format!("Page {p} of {n}")))
}

/// Timeline entries: events, changes and notes, in time order.
pub fn timeline(s: &Session) -> Vec<(u64, String, Option<Rgb>)> {
    let mut v: Vec<(u64, String, Option<Rgb>)> = Vec::new();
    for e in &s.events {
        let c = match e.kind.as_str() {
            "offline" | "finding" => Some(amber()),
            "pause" | "resume" => Some(muted()),
            _ => None,
        };
        v.push((e.ts_ms, e.text.clone(), c));
    }
    for c in &s.changes {
        v.push((c.ts_ms, format!("{} · {}: {} → {} ({})", c.device, c.setting, c.before.clone().unwrap_or_else(|| "—".into()), c.after.clone().unwrap_or_else(|| "—".into()), result_text(c).to_lowercase()), Some(result_color(&c.result))));
    }
    for n in &s.notes {
        v.push((n.ts_ms, format!("Note: {}", n.text), None));
    }
    v.sort_by_key(|x| x.0);
    v
}

// ------------------------------------------------------------------ HTML

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

pub fn tech_html(s: &Session, files: &[(String, u64, Option<String>)]) -> String {
    let end = s.ended_ms.unwrap_or(s.started_ms);
    let mut h = String::new();
    h += "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">";
    h += &format!("<title>{} – technician report {}</title>", esc(&s.vessel.display_name()), esc(&s.id));
    h += r#"<style>
:root{--ink:#0D1726;--muted:#566173;--soft:#4A5568;--line:#D5DAE1;--bg:#fff;--tint:#F6F7F9;--green:#13804B;--amber:#9A5100;--red:#C4302B}
@media (prefers-color-scheme:dark){:root{--ink:#E6EAF0;--muted:#9AA3B2;--soft:#B6BFCC;--line:#2A3446;--bg:#0E1420;--tint:#162033;--green:#4CC38A;--amber:#E0A050;--red:#F07068}}
body{margin:0;background:var(--bg);color:var(--ink);font:14px/1.5 Geist,system-ui,-apple-system,"Segoe UI",sans-serif}
main{max-width:980px;margin:0 auto;padding:32px 20px 48px}
.k{margin:0;font-size:12px;font-weight:600;letter-spacing:.08em;text-transform:uppercase;color:var(--muted)}
h1{margin:2px 0 4px;font-size:26px;letter-spacing:.02em}
h2{font-size:16px;margin:28px 0 8px}
header{border-bottom:2px solid var(--ink);padding-bottom:12px;display:flex;flex-wrap:wrap;justify-content:space-between;gap:12px}
dl{display:grid;grid-template-columns:repeat(auto-fit,minmax(220px,1fr));gap:4px 24px;margin:14px 0 0}
dl div{display:flex;gap:10px}dt{color:var(--muted);min-width:80px}dd{margin:0}
.mono{font-family:"Geist Mono",ui-monospace,monospace}
.scroll{overflow-x:auto}
table{width:100%;border-collapse:collapse;font-size:13px}th{text-align:left;color:var(--soft);font-weight:600;border-bottom:1px solid var(--ink);padding:6px 8px 6px 0}
td{border-bottom:1px solid var(--line);padding:6px 8px 6px 0;vertical-align:top}
.ok{color:var(--green)}.warn{color:var(--amber)}.bad{color:var(--red)}.mut{color:var(--muted)}
.note{background:var(--tint);border-radius:8px;padding:8px 12px;margin:6px 0}
ul{padding-left:18px;margin:0}li{margin:4px 0}
footer{margin-top:32px;border-top:1px solid var(--line);padding-top:8px;font-size:12px;color:var(--muted)}
@media print{body{background:#fff;color:#000}.scroll{overflow:visible}}
</style></head><body><main>"#;
    h += &format!(
        "<header><div><p class=\"k\">Technician report</p><h1>{}</h1></div><div class=\"mut\" style=\"text-align:right\">{}<br>{} – {} {} ({})</div></header>",
        esc(&s.vessel.display_name()),
        esc(&local_fmt(s.started_ms, "%a %-d %b %Y")),
        local_fmt(s.started_ms, "%H:%M:%S"),
        local_fmt(end, "%H:%M:%S"),
        esc(&utc_offset_label(s.started_ms)),
        esc(&fmt_duration(end.saturating_sub(s.started_ms)))
    );
    h += "<dl>";
    for (k, v) in header_facts(s) {
        h += &format!("<div><dt>{}</dt><dd>{}</dd></div>", esc(&k), esc(&v));
    }
    h += "</dl>";
    let settings: Vec<&ss::Change> = s.changes.iter().filter(|c| c.kind == "setting").collect();
    h += &format!("<h2>Changes sent ({})</h2>", settings.iter().filter(|c| c.result != "failed").count());
    if settings.is_empty() {
        h += "<p class=\"mut\">No settings were changed.</p>";
    } else {
        h += "<div class=\"scroll\"><table><thead><tr><th>Time</th><th>Device</th><th>Setting</th><th>Before</th><th>After</th><th>How</th><th>Result</th></tr></thead><tbody>";
        for c in &settings {
            let cls = match c.result.as_str() {
                "confirmed" => "ok",
                "failed" => "bad",
                _ => "warn",
            };
            h += &format!(
                "<tr><td class=\"mono\">{}</td><td>{}{}</td><td>{}</td><td class=\"mono\">{}</td><td class=\"mono\">{}</td><td class=\"mut\">{}</td><td class=\"{cls}\">{}</td></tr>",
                local_fmt(c.ts_ms, "%H:%M:%S"),
                esc(&c.device),
                c.serial.as_ref().map(|x| format!("<br><span class=\"mut\">s/n {}</span>", esc(x))).unwrap_or_default(),
                esc(&c.setting),
                esc(&c.before.clone().unwrap_or_else(|| "—".into())),
                esc(&c.after.clone().unwrap_or_else(|| "—".into())),
                esc(&c.method),
                esc(&result_text(c))
            );
        }
        h += "</tbody></table></div>";
    }
    let other: Vec<&ss::Change> = s.changes.iter().filter(|c| c.kind != "setting").collect();
    if !other.is_empty() {
        h += "<h2>Control and expert commands</h2><div class=\"scroll\"><table><thead><tr><th>Time</th><th>Target</th><th>Command</th><th>Value</th><th>Result</th></tr></thead><tbody>";
        for c in other {
            h += &format!("<tr><td class=\"mono\">{}</td><td>{}</td><td>{}</td><td class=\"mono\">{}</td><td>{}</td></tr>", local_fmt(c.ts_ms, "%H:%M:%S"), esc(&c.device), esc(&c.setting), esc(&c.after.clone().unwrap_or_default()), esc(&result_text(c)));
        }
        h += "</tbody></table></div>";
    }
    h += "<h2>Findings</h2><ul>";
    let (fixed, open, off) = (s.fixed(), s.open(), offline_at_end(s));
    if fixed.is_empty() && open.is_empty() && off.is_empty() {
        h += "<li class=\"mut\">No problems found.</li>";
    }
    for f in fixed {
        h += &format!("<li><strong class=\"ok\">Fixed:</strong> {} <span class=\"mut\">(found {}, last seen {})</span></li>", esc(&f.message), local_fmt(f.first_ms, "%H:%M"), local_fmt(f.last_ms, "%H:%M"));
    }
    for f in open {
        h += &format!("<li><strong class=\"warn\">Open:</strong> {}{}</li>", esc(&f.message), f.suggestion.as_ref().map(|x| format!(" — {}", esc(x))).unwrap_or_default());
    }
    for x in off {
        h += &format!("<li><strong class=\"warn\">Open:</strong> {} not responding at the end of the session.</li>", esc(&x.label()));
    }
    h += "</ul>";
    if let Some(p) = &s.previous {
        h += &format!("<h2>Since last visit ({})</h2><ul>", esc(&local_fmt(p.started_ms, "%-d %b %Y")));
        if s.since_last.is_empty() {
            h += "<li class=\"mut\">No changes to devices, software or settings.</li>";
        }
        for i in &s.since_last {
            h += &format!("<li><strong>{}:</strong> {}</li>", esc(&title_kind(&i.kind)), esc(&i.text));
        }
        h += "</ul>";
    }
    h += "<h2>Bus over the session</h2><div class=\"scroll\"><table><tbody>";
    for (k, v) in bus_facts(s, files) {
        h += &format!("<tr><td class=\"mut\" style=\"width:32%\">{}</td><td>{}</td></tr>", esc(&k), esc(&v));
    }
    h += "</tbody></table></div>";
    if !s.notes.is_empty() {
        h += "<h2>Notes</h2>";
        for n in &s.notes {
            h += &format!("<p class=\"note\"><span class=\"mono mut\">{}</span> · {}</p>", local_fmt(n.ts_ms, "%H:%M"), esc(&n.text));
        }
    }
    h += "<h2>Timeline</h2><div class=\"scroll\"><table><tbody>";
    for (t, txt, _) in timeline(s) {
        h += &format!("<tr><td class=\"mono mut\" style=\"width:90px\">{}</td><td>{}</td></tr>", local_fmt(t, "%H:%M:%S"), esc(&txt));
    }
    h += "</tbody></table></div>";
    h += "<h2>Devices at the end of the session</h2><div class=\"scroll\"><table><thead><tr><th>Addr</th><th>Model</th><th>Maker</th><th>Serial</th><th>Software</th><th>Instance</th><th>Data instances</th><th>Installation</th><th>LEN</th><th>Status</th></tr></thead><tbody>";
    for x in &s.devices_end {
        let inst: Vec<String> = x.instances.iter().map(|(p, k)| format!("{p}: {}", k.join("; "))).collect();
        h += &format!(
            "<tr><td class=\"mono\">{}</td><td>{}</td><td class=\"mut\">{}</td><td class=\"mono\">{}</td><td class=\"mono\">{}</td><td class=\"mono\">{}</td><td class=\"mut\">{}</td><td>{}</td><td class=\"mono\">{}</td><td class=\"{}\">{}</td></tr>",
            x.address.map(|a| a.to_string()).unwrap_or_default(),
            esc(&x.model.clone().unwrap_or_default()),
            esc(&x.manufacturer.clone().unwrap_or_default()),
            esc(&x.serial.clone().unwrap_or_default()),
            esc(&x.software.clone().unwrap_or_default()),
            x.device_instance.map(|i| i.to_string()).unwrap_or_default(),
            esc(&inst.join("; ")),
            esc(&x.install1.clone().unwrap_or_default()),
            x.len.map(|l| l.to_string()).unwrap_or_default(),
            if x.online { "ok" } else { "warn" },
            if x.online { "online" } else { "offline" }
        );
    }
    h += "</tbody></table></div><h2>Files in this session</h2><ul class=\"mono\" style=\"font-size:12px\">";
    for (f, size, sha) in files {
        h += &format!("<li>{} · {:.1} MB · sha256 {}</li>", esc(f), *size as f64 / 1e6, esc(&sha.clone().unwrap_or_default()));
    }
    h += "<li>boat-before.keelwire · boat-after.keelwire · changes.csv · devices.csv · pgns.csv · session.json</li></ul>";
    h += &format!("<footer>Keelwire {} · other vessels’ AIS omitted from this report</footer></main></body></html>", esc(&s.app_version));
    h
}

fn title_kind(k: &str) -> String {
    match k {
        "added" => "Added",
        "gone" => "Gone",
        "software" => "Software",
        _ => "Setting",
    }
    .into()
}

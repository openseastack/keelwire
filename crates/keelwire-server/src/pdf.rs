//! Minimal PDF writer for service reports: pages, embedded TrueType fonts
//! (Identity-H, with ToUnicode so text can be searched and copied), text,
//! rectangles and lines. Coordinates are points from the top-left corner.

#![allow(clippy::too_many_arguments)]

use std::collections::BTreeMap;
use std::io::Write;

use flate2::write::ZlibEncoder;
use flate2::Compression;

pub const LETTER: (f32, f32) = (612.0, 792.0);

pub type Rgb = (f32, f32, f32);

pub fn hex_rgb(h: u32) -> Rgb {
    (((h >> 16) & 0xFF) as f32 / 255.0, ((h >> 8) & 0xFF) as f32 / 255.0, (h & 0xFF) as f32 / 255.0)
}

pub struct Font {
    data: &'static [u8],
    upem: f32,
    ascender: f32,
    descender: f32,
    /// char → (glyph id, advance in font units)
    cmap: BTreeMap<char, (u16, u16)>,
    /// glyphs actually used: gid → (advance, char)
    used: BTreeMap<u16, (u16, char)>,
    notdef_adv: u16,
}

impl Font {
    pub fn load(data: &'static [u8]) -> Result<Font, String> {
        let face = ttf_parser::Face::parse(data, 0).map_err(|e| format!("font: {e}"))?;
        let upem = face.units_per_em() as f32;
        let mut cmap = BTreeMap::new();
        if let Some(t) = face.tables().cmap {
            for st in t.subtables.into_iter().filter(|s| s.is_unicode()) {
                st.codepoints(|cp| {
                    if let (Some(c), Some(g)) = (char::from_u32(cp), st.glyph_index(cp)) {
                        cmap.entry(c).or_insert((g.0, face.glyph_hor_advance(g).unwrap_or(0)));
                    }
                });
            }
        }
        let notdef_adv = face.glyph_hor_advance(ttf_parser::GlyphId(0)).unwrap_or(500);
        Ok(Font { data, upem, ascender: face.ascender() as f32, descender: face.descender() as f32, cmap, used: BTreeMap::new(), notdef_adv })
    }

    fn glyph(&self, c: char) -> (u16, u16) {
        if let Some(g) = self.cmap.get(&c) {
            return *g;
        }
        // Fallbacks for characters outside the embedded subset.
        let alt = match c {
            '→' => '>',
            '✓' | '✔' => '+',
            '•' => '·',
            '\u{2009}' | '\u{202f}' | '\u{a0}' => ' ',
            _ => '?',
        };
        self.cmap.get(&alt).copied().unwrap_or((0, self.notdef_adv))
    }

    pub fn width(&self, size: f32, s: &str) -> f32 {
        s.chars().map(|c| self.glyph(c).1 as f32).sum::<f32>() * size / self.upem
    }

    pub fn ascent(&self, size: f32) -> f32 {
        self.ascender * size / self.upem
    }

    pub fn descent(&self, size: f32) -> f32 {
        -self.descender * size / self.upem
    }

    fn encode(&mut self, s: &str) -> String {
        let mut out = String::with_capacity(s.len() * 4 + 2);
        out.push('<');
        for c in s.chars() {
            let (g, adv) = self.glyph(c);
            let shown = if self.cmap.contains_key(&c) { c } else { '?' };
            self.used.entry(g).or_insert((adv, shown));
            out.push_str(&format!("{g:04X}"));
        }
        out.push('>');
        out
    }
}

pub struct Pdf {
    pub size: (f32, f32),
    pub fonts: Vec<Font>,
    pages: Vec<Vec<u8>>,
    pub title: String,
}

fn n(v: f32) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

impl Pdf {
    pub fn new(size: (f32, f32), fonts: Vec<Font>, title: &str) -> Pdf {
        Pdf { size, fonts, pages: Vec::new(), title: title.into() }
    }

    pub fn add_page(&mut self) -> usize {
        self.pages.push(Vec::new());
        self.pages.len() - 1
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    fn ops(&mut self, page: usize, s: &str) {
        self.pages[page].extend_from_slice(s.as_bytes());
        self.pages[page].push(b'\n');
    }

    /// Text with its baseline at `y` (from the top).
    pub fn text(&mut self, page: usize, x: f32, y: f32, font: usize, size: f32, color: Rgb, s: &str) {
        if s.is_empty() {
            return;
        }
        let enc = self.fonts[font].encode(s);
        let yy = self.size.1 - y;
        let op = format!("BT {} {} {} rg /F{} {} Tf {} {} Td {} Tj ET", n(color.0), n(color.1), n(color.2), font, n(size), n(x), n(yy), enc);
        self.ops(page, &op);
    }

    pub fn rect(&mut self, page: usize, x: f32, y: f32, w: f32, h: f32, fill: Option<Rgb>, stroke: Option<(Rgb, f32)>) {
        let yy = self.size.1 - y - h;
        let mut op = String::from("q ");
        if let Some(c) = fill {
            op += &format!("{} {} {} rg ", n(c.0), n(c.1), n(c.2));
        }
        if let Some((c, lw)) = stroke {
            op += &format!("{} {} {} RG {} w ", n(c.0), n(c.1), n(c.2), n(lw));
        }
        op += &format!("{} {} {} {} re ", n(x), n(yy), n(w), n(h));
        op += match (fill.is_some(), stroke.is_some()) {
            (true, true) => "B",
            (true, false) => "f",
            (false, true) => "S",
            _ => "n",
        };
        op += " Q";
        self.ops(page, &op);
    }

    /// Rounded rectangle (radius r).
    pub fn round_rect(&mut self, page: usize, x: f32, y: f32, w: f32, h: f32, r: f32, fill: Option<Rgb>, stroke: Option<(Rgb, f32)>) {
        let r = r.min(w / 2.0).min(h / 2.0);
        let k = 0.5523 * r;
        let (x0, x1) = (x, x + w);
        let (y0, y1) = (self.size.1 - y - h, self.size.1 - y); // bottom, top in PDF space
        let mut op = String::from("q ");
        if let Some(c) = fill {
            op += &format!("{} {} {} rg ", n(c.0), n(c.1), n(c.2));
        }
        if let Some((c, lw)) = stroke {
            op += &format!("{} {} {} RG {} w ", n(c.0), n(c.1), n(c.2), n(lw));
        }
        op += &format!("{} {} m ", n(x0 + r), n(y0));
        op += &format!("{} {} l ", n(x1 - r), n(y0));
        op += &format!("{} {} {} {} {} {} c ", n(x1 - r + k), n(y0), n(x1), n(y0 + r - k), n(x1), n(y0 + r));
        op += &format!("{} {} l ", n(x1), n(y1 - r));
        op += &format!("{} {} {} {} {} {} c ", n(x1), n(y1 - r + k), n(x1 - r + k), n(y1), n(x1 - r), n(y1));
        op += &format!("{} {} l ", n(x0 + r), n(y1));
        op += &format!("{} {} {} {} {} {} c ", n(x0 + r - k), n(y1), n(x0), n(y1 - r + k), n(x0), n(y1 - r));
        op += &format!("{} {} l ", n(x0), n(y0 + r));
        op += &format!("{} {} {} {} {} {} c h ", n(x0), n(y0 + r - k), n(x0 + r - k), n(y0), n(x0 + r), n(y0));
        op += match (fill.is_some(), stroke.is_some()) {
            (true, true) => "B",
            (true, false) => "f",
            (false, true) => "S",
            _ => "n",
        };
        op += " Q";
        self.ops(page, &op);
    }

    pub fn line(&mut self, page: usize, x0: f32, y0: f32, x1: f32, y1: f32, color: Rgb, width: f32) {
        let h = self.size.1;
        let op = format!("q {} {} {} RG {} w {} {} m {} {} l S Q", n(color.0), n(color.1), n(color.2), n(width), n(x0), n(h - y0), n(x1), n(h - y1));
        self.ops(page, &op);
    }

    pub fn finish(self) -> Vec<u8> {
        let mut objs: Vec<Vec<u8>> = Vec::new(); // object i+1
        let add = |o: Vec<u8>, objs: &mut Vec<Vec<u8>>| -> usize {
            objs.push(o);
            objs.len()
        };
        let reserve = |objs: &mut Vec<Vec<u8>>| -> usize {
            objs.push(Vec::new());
            objs.len()
        };
        let catalog = reserve(&mut objs);
        let pages_id = reserve(&mut objs);
        // Fonts
        let mut font_ids = Vec::new();
        for f in &self.fonts {
            let ff = add(stream(&[], f.data, true), &mut objs);
            let scale = 1000.0 / f.upem;
            let desc = add(
                format!(
                    "<< /Type /FontDescriptor /FontName /KW{0}+Font /Flags 32 /FontBBox [-200 {1} 1200 {2}] /ItalicAngle 0 /Ascent {2} /Descent {1} /CapHeight 700 /StemV 80 /FontFile2 {3} 0 R >>",
                    font_ids.len(),
                    (f.descender * scale) as i32,
                    (f.ascender * scale) as i32,
                    ff
                )
                .into_bytes(),
                &mut objs,
            );
            let mut w = String::from("[");
            for (g, (adv, _)) in &f.used {
                w += &format!("{} [{}] ", g, (*adv as f32 * scale).round() as i32);
            }
            w += "]";
            let cid = add(
                format!("<< /Type /Font /Subtype /CIDFontType2 /BaseFont /KW{}+Font /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor {} 0 R /DW {} /W {} /CIDToGIDMap /Identity >>", font_ids.len(), desc, (f.notdef_adv as f32 * scale) as i32, w).into_bytes(),
                &mut objs,
            );
            let mut cm = String::from("/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def /CMapName /Adobe-Identity-UCS def /CMapType 2 def 1 begincodespacerange <0000> <FFFF> endcodespacerange\n");
            let entries: Vec<_> = f.used.iter().collect();
            for chunk in entries.chunks(100) {
                cm += &format!("{} beginbfchar\n", chunk.len());
                for (g, (_, c)) in chunk {
                    let mut u = [0u16; 2];
                    let units = c.encode_utf16(&mut u);
                    cm += &format!("<{:04X}> <{}>\n", g, units.iter().map(|x| format!("{x:04X}")).collect::<String>());
                }
                cm += "endbfchar\n";
            }
            cm += "endcmap CMapName currentdict /CMap defineresource pop end end";
            let tu = add(stream(&[], cm.as_bytes(), true), &mut objs);
            let t0 = add(format!("<< /Type /Font /Subtype /Type0 /BaseFont /KW{}+Font /Encoding /Identity-H /DescendantFonts [{} 0 R] /ToUnicode {} 0 R >>", font_ids.len(), cid, tu).into_bytes(), &mut objs);
            font_ids.push(t0);
        }
        let font_dict: String = font_ids.iter().enumerate().map(|(i, id)| format!("/F{i} {id} 0 R ")).collect();
        let mut page_ids = Vec::new();
        for content in &self.pages {
            let c = add(stream(&[], content, true), &mut objs);
            let p = add(
                format!("<< /Type /Page /Parent {} 0 R /MediaBox [0 0 {} {}] /Resources << /Font << {}>> >> /Contents {} 0 R >>", pages_id, n(self.size.0), n(self.size.1), font_dict, c).into_bytes(),
                &mut objs,
            );
            page_ids.push(p);
        }
        objs[catalog - 1] = format!("<< /Type /Catalog /Pages {pages_id} 0 R >>").into_bytes();
        objs[pages_id - 1] = format!("<< /Type /Pages /Kids [{}] /Count {} >>", page_ids.iter().map(|p| format!("{p} 0 R")).collect::<Vec<_>>().join(" "), page_ids.len()).into_bytes();
        let info = add(format!("<< /Title {} /Producer (Keelwire {}) >>", pdf_text(&self.title), crate::VERSION).into_bytes(), &mut objs);

        let mut out: Vec<u8> = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
        let mut offsets = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            out.extend_from_slice(o);
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
        for o in offsets {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(format!("trailer\n<< /Size {} /Root {} 0 R /Info {} 0 R >>\nstartxref\n{}\n%%EOF\n", objs.len() + 1, catalog, info, xref).as_bytes());
        out
    }
}

/// PDF text string (UTF-16BE with BOM, hex).
fn pdf_text(s: &str) -> String {
    let mut h = String::from("<FEFF");
    for u in s.encode_utf16() {
        h += &format!("{u:04X}");
    }
    h + ">"
}

fn stream(extra_dict: &[u8], data: &[u8], compress: bool) -> Vec<u8> {
    let body = if compress {
        let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
        let _ = e.write_all(data);
        e.finish().unwrap_or_default()
    } else {
        data.to_vec()
    };
    let mut o = Vec::with_capacity(body.len() + 80);
    o.extend_from_slice(b"<< /Length ");
    o.extend_from_slice(body.len().to_string().as_bytes());
    if compress {
        o.extend_from_slice(b" /Filter /FlateDecode");
    }
    if !extra_dict.is_empty() {
        o.push(b' ');
        o.extend_from_slice(extra_dict);
    }
    if compress && extra_dict.is_empty() {
        // FontFile2 needs Length1 (uncompressed size); harmless elsewhere.
        o.extend_from_slice(format!(" /Length1 {}", data.len()).as_bytes());
    }
    o.extend_from_slice(b" >>\nstream\n");
    o.extend_from_slice(&body);
    o.extend_from_slice(b"\nendstream");
    o
}

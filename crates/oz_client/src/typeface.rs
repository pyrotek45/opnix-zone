//! The game's own typeface, built in code like everything else.
//!
//! Each glyph is drawn below as a 7-row pixel grid (`#` = ink). At startup the
//! grids are turned into a real TrueType font - square pixels merged into
//! runs, one rectangle contour per run - and swapped in as Bevy's default
//! font, so every label in every screen uses it without naming it.
//! Lower-case letters share the capitals: the game speaks in capitals.

use bevy::prelude::*;

/// Glyphs: rows top to bottom; an optional eighth row hangs below the baseline.
const GLYPHS: &[(char, &[&str])] = &[
    ('A', &[".###.", "#...#", "#...#", "#####", "#...#", "#...#", "#...#"]),
    ('B', &["####.", "#...#", "#...#", "####.", "#...#", "#...#", "####."]),
    ('C', &[".####", "#....", "#....", "#....", "#....", "#....", ".####"]),
    ('D', &["###..", "#..#.", "#...#", "#...#", "#...#", "#..#.", "###.."]),
    ('E', &["#####", "#....", "#....", "####.", "#....", "#....", "#####"]),
    ('F', &["#####", "#....", "#....", "####.", "#....", "#....", "#...."]),
    ('G', &[".####", "#....", "#....", "#.###", "#...#", "#...#", ".###."]),
    ('H', &["#...#", "#...#", "#...#", "#####", "#...#", "#...#", "#...#"]),
    ('I', &["###", ".#.", ".#.", ".#.", ".#.", ".#.", "###"]),
    ('J', &["..###", "...#.", "...#.", "...#.", "...#.", "#..#.", ".##.."]),
    ('K', &["#...#", "#..#.", "#.#..", "##...", "#.#..", "#..#.", "#...#"]),
    ('L', &["#....", "#....", "#....", "#....", "#....", "#....", "#####"]),
    ('M', &["#...#", "##.##", "#.#.#", "#.#.#", "#...#", "#...#", "#...#"]),
    ('N', &["#...#", "##..#", "#.#.#", "#..##", "#...#", "#...#", "#...#"]),
    ('O', &[".###.", "#...#", "#...#", "#...#", "#...#", "#...#", ".###."]),
    ('P', &["####.", "#...#", "#...#", "####.", "#....", "#....", "#...."]),
    ('Q', &[".###.", "#...#", "#...#", "#...#", "#.#.#", "#..#.", ".##.#"]),
    ('R', &["####.", "#...#", "#...#", "####.", "#.#..", "#..#.", "#...#"]),
    ('S', &[".####", "#....", "#....", ".###.", "....#", "....#", "####."]),
    ('T', &["#####", "..#..", "..#..", "..#..", "..#..", "..#..", "..#.."]),
    ('U', &["#...#", "#...#", "#...#", "#...#", "#...#", "#...#", ".###."]),
    ('V', &["#...#", "#...#", "#...#", "#...#", ".#.#.", ".#.#.", "..#.."]),
    ('W', &["#...#", "#...#", "#...#", "#.#.#", "#.#.#", "##.##", "#...#"]),
    ('X', &["#...#", "#...#", ".#.#.", "..#..", ".#.#.", "#...#", "#...#"]),
    ('Y', &["#...#", "#...#", ".#.#.", "..#..", "..#..", "..#..", "..#.."]),
    ('Z', &["#####", "....#", "...#.", "..#..", ".#...", "#....", "#####"]),
    ('0', &[".###.", "#...#", "#..##", "#.#.#", "##..#", "#...#", ".###."]),
    ('1', &[".#.", "##.", ".#.", ".#.", ".#.", ".#.", "###"]),
    ('2', &[".###.", "#...#", "....#", "...#.", "..#..", ".#...", "#####"]),
    ('3', &["####.", "....#", "....#", ".###.", "....#", "....#", "####."]),
    ('4', &["...#.", "..##.", ".#.#.", "#..#.", "#####", "...#.", "...#."]),
    ('5', &["#####", "#....", "####.", "....#", "....#", "#...#", ".###."]),
    ('6', &[".###.", "#....", "#....", "####.", "#...#", "#...#", ".###."]),
    ('7', &["#####", "....#", "...#.", "..#..", ".#...", ".#...", ".#..."]),
    ('8', &[".###.", "#...#", "#...#", ".###.", "#...#", "#...#", ".###."]),
    ('9', &[".###.", "#...#", "#...#", ".####", "....#", "....#", ".###."]),
    ('.', &[".", ".", ".", ".", ".", ".", "#"]),
    (',', &[".", ".", ".", ".", ".", ".", "#", "#"]),
    (':', &[".", ".", "#", ".", ".", "#", "."]),
    (';', &[".", ".", "#", ".", ".", "#", "#", "#"]),
    ('!', &["#", "#", "#", "#", "#", ".", "#"]),
    ('?', &[".###.", "#...#", "....#", "...#.", "..#..", ".....", "..#.."]),
    ('-', &["....", "....", "....", "####", "....", "....", "...."]),
    ('+', &[".....", "..#..", "..#..", "#####", "..#..", "..#..", "....."]),
    ('=', &["....", "....", "####", "....", "####", "....", "...."]),
    ('/', &["....#", "...#.", "...#.", "..#..", ".#...", ".#...", "#...."]),
    ('\\', &["#....", ".#...", ".#...", "..#..", "...#.", "...#.", "....#"]),
    ('(', &[".#", "#.", "#.", "#.", "#.", "#.", ".#"]),
    (')', &["#.", ".#", ".#", ".#", ".#", ".#", "#."]),
    ('[', &["##", "#.", "#.", "#.", "#.", "#.", "##"]),
    (']', &["##", ".#", ".#", ".#", ".#", ".#", "##"]),
    ('<', &["...#", "..#.", ".#..", "#...", ".#..", "..#.", "...#"]),
    ('>', &["#...", ".#..", "..#.", "...#", "..#.", ".#..", "#..."]),
    ('%', &["##..#", "##..#", "...#.", "..#..", ".#...", "#..##", "#..##"]),
    ('\'', &["#", "#", ".", ".", ".", ".", "."]),
    ('"', &["#.#", "#.#", "...", "...", "...", "...", "..."]),
    ('_', &[".....", ".....", ".....", ".....", ".....", ".....", "#####"]),
    ('|', &["#", "#", "#", "#", "#", "#", "#"]),
    ('*', &[".....", "#.#.#", ".###.", "#####", ".###.", "#.#.#", "....."]),
    ('#', &[".#.#.", "#####", ".#.#.", ".#.#.", ".#.#.", "#####", ".#.#."]),
    ('$', &["..#..", ".####", "#.#..", ".###.", "..#.#", "####.", "..#.."]),
    ('&', &[".##..", "#..#.", "#.#..", ".#...", "#.#.#", "#..#.", ".##.#"]),
    ('@', &[".###.", "#...#", "#.###", "#.#.#", "#.###", "#....", ".###."]),
    ('^', &["..#..", ".#.#.", "#...#", ".....", ".....", ".....", "....."]),
    ('~', &[".....", ".....", ".#..#", "#.##.", ".....", ".....", "....."]),
    ('`', &["#.", ".#", "..", "..", "..", "..", ".."]),
    ('{', &["..#", ".#.", ".#.", "#..", ".#.", ".#.", "..#"]),
    ('}', &["#..", ".#.", ".#.", "..#", ".#.", ".#.", "#.."]),
];

/// Font units per pixel of a glyph, and per em.
const PX: i16 = 100;
const EM: u16 = 1000;

struct Glyph {
    advance: u16,
    /// Rectangles (x0, y0, x1, y1) in font units, y up from the baseline.
    rects: Vec<(i16, i16, i16, i16)>,
}

fn glyph(rows: &[&str]) -> Glyph {
    let width = rows.iter().map(|r| r.len()).max().unwrap_or(0) as i16;
    let mut rects = Vec::new();
    for (r, row) in rows.iter().enumerate() {
        // Row 0 is the top of the capitals (7 pixels tall); row 7 hangs below the baseline.
        let top = (7 - r as i16) * PX;
        let bytes = row.as_bytes();
        let mut c = 0;
        while c < bytes.len() {
            if bytes[c] != b'#' {
                c += 1;
                continue;
            }
            let start = c;
            while c < bytes.len() && bytes[c] == b'#' {
                c += 1;
            }
            rects.push((start as i16 * PX, top - PX, c as i16 * PX, top));
        }
    }
    Glyph { advance: ((width + 1) * PX) as u16, rects }
}

/// A TrueType file for the glyphs above.
pub fn build() -> Vec<u8> {
    // Glyph 0 is .notdef (a hollow box), 1 is the space, then the drawn glyphs.
    let mut glyphs = vec![
        Glyph { advance: 600, rects: vec![(0, 0, 500, 100), (0, 600, 500, 700), (0, 100, 100, 600), (400, 100, 500, 600)] },
        Glyph { advance: 400, rects: Vec::new() },
    ];
    let mut map: Vec<(u16, u16)> = vec![(32, 1)];
    for (ch, rows) in GLYPHS {
        let id = glyphs.len() as u16;
        glyphs.push(glyph(rows));
        map.push((*ch as u16, id));
        if ch.is_ascii_uppercase() {
            map.push((ch.to_ascii_lowercase() as u16, id));
        }
    }
    map.push((0xA0, 1));
    map.sort();

    // glyf + loca.
    let mut glyf: Vec<u8> = Vec::new();
    let mut loca: Vec<u32> = Vec::new();
    let (mut max_points, mut max_contours) = (0u16, 0u16);
    let (mut x_min, mut y_min, mut x_max, mut y_max) = (i16::MAX, i16::MAX, i16::MIN, i16::MIN);
    for g in &glyphs {
        loca.push(glyf.len() as u32);
        if g.rects.is_empty() {
            continue;
        }
        let gx0 = g.rects.iter().map(|r| r.0).min().unwrap();
        let gy0 = g.rects.iter().map(|r| r.1).min().unwrap();
        let gx1 = g.rects.iter().map(|r| r.2).max().unwrap();
        let gy1 = g.rects.iter().map(|r| r.3).max().unwrap();
        (x_min, y_min, x_max, y_max) = (x_min.min(gx0), y_min.min(gy0), x_max.max(gx1), y_max.max(gy1));
        let contours = g.rects.len() as u16;
        max_contours = max_contours.max(contours);
        max_points = max_points.max(contours * 4);
        put_i16(&mut glyf, contours as i16);
        for v in [gx0, gy0, gx1, gy1] {
            put_i16(&mut glyf, v);
        }
        for i in 0..contours {
            put_u16(&mut glyf, i * 4 + 3);
        }
        put_u16(&mut glyf, 0); // no instructions
        glyf.extend(std::iter::repeat_n(1u8, contours as usize * 4)); // every point on the curve
        // Clockwise (outer contour, y up): bottom-left, top-left, top-right, bottom-right.
        let points: Vec<(i16, i16)> = g.rects.iter().flat_map(|&(x0, y0, x1, y1)| [(x0, y0), (x0, y1), (x1, y1), (x1, y0)]).collect();
        let mut last = 0;
        for (x, _) in &points {
            put_i16(&mut glyf, x - last);
            last = *x;
        }
        let mut last = 0;
        for (_, y) in &points {
            put_i16(&mut glyf, y - last);
            last = *y;
        }
        while glyf.len() % 4 != 0 {
            glyf.push(0);
        }
    }
    loca.push(glyf.len() as u32);
    let mut loca_bytes = Vec::new();
    for offset in &loca {
        put_u32(&mut loca_bytes, *offset);
    }
    let count = glyphs.len() as u16;
    let advance_max = glyphs.iter().map(|g| g.advance).max().unwrap_or(0);

    let mut head = Vec::new();
    put_u32(&mut head, 0x0001_0000);
    put_u32(&mut head, 0x0001_0000);
    put_u32(&mut head, 0); // checksum adjustment, filled in last
    put_u32(&mut head, 0x5F0F_3CF5);
    put_u16(&mut head, 0b1011);
    put_u16(&mut head, EM);
    head.extend([0u8; 16]); // created, modified
    for v in [x_min, y_min, x_max, y_max] {
        put_i16(&mut head, v);
    }
    put_u16(&mut head, 0); // mac style
    put_u16(&mut head, 6); // smallest readable size
    put_i16(&mut head, 2);
    put_i16(&mut head, 1); // long loca
    put_i16(&mut head, 0);

    let mut hhea = Vec::new();
    put_u32(&mut hhea, 0x0001_0000);
    put_i16(&mut hhea, 900);
    put_i16(&mut hhea, -250);
    put_i16(&mut hhea, 100);
    put_u16(&mut hhea, advance_max);
    put_i16(&mut hhea, 0);
    put_i16(&mut hhea, 0);
    put_i16(&mut hhea, x_max);
    put_i16(&mut hhea, 1);
    put_i16(&mut hhea, 0);
    put_i16(&mut hhea, 0);
    hhea.extend([0u8; 8]);
    put_i16(&mut hhea, 0);
    put_u16(&mut hhea, count);

    let mut hmtx = Vec::new();
    for g in &glyphs {
        put_u16(&mut hmtx, g.advance);
        put_i16(&mut hmtx, g.rects.iter().map(|r| r.0).min().unwrap_or(0));
    }

    let mut maxp = Vec::new();
    put_u32(&mut maxp, 0x0001_0000);
    put_u16(&mut maxp, count);
    put_u16(&mut maxp, max_points);
    put_u16(&mut maxp, max_contours);
    put_u16(&mut maxp, 0);
    put_u16(&mut maxp, 0);
    put_u16(&mut maxp, 2);
    maxp.extend([0u8; 16]);

    // cmap: one format 4 subtable (Unicode BMP), shared by the Unicode and Windows platforms.
    let mut segments: Vec<(u16, u16)> = map.clone();
    segments.push((0xFFFF, 0));
    let seg_count = segments.len() as u16;
    let mut sub = Vec::new();
    put_u16(&mut sub, 4);
    put_u16(&mut sub, 0); // length, patched below
    put_u16(&mut sub, 0);
    put_u16(&mut sub, seg_count * 2);
    let search = 2 * (1u16 << (15 - seg_count.leading_zeros() as u16));
    put_u16(&mut sub, search);
    put_u16(&mut sub, (search / 2).trailing_zeros() as u16);
    put_u16(&mut sub, seg_count * 2 - search);
    for (code, _) in &segments {
        put_u16(&mut sub, *code);
    }
    put_u16(&mut sub, 0);
    for (code, _) in &segments {
        put_u16(&mut sub, *code);
    }
    for (code, id) in &segments {
        put_u16(&mut sub, if *code == 0xFFFF { 1 } else { id.wrapping_sub(*code) });
    }
    for _ in &segments {
        put_u16(&mut sub, 0);
    }
    let length = sub.len() as u16;
    sub[2..4].copy_from_slice(&length.to_be_bytes());
    let mut cmap = Vec::new();
    put_u16(&mut cmap, 0);
    put_u16(&mut cmap, 2);
    for (platform, encoding) in [(0u16, 3u16), (3, 1)] {
        put_u16(&mut cmap, platform);
        put_u16(&mut cmap, encoding);
        put_u32(&mut cmap, 20);
    }
    cmap.extend(sub);

    let mut name = Vec::new();
    let strings = [(1u16, "Opnix Pixel"), (2, "Regular"), (3, "Opnix Pixel Regular"), (4, "Opnix Pixel"), (5, "Version 1.0"), (6, "OpnixPixel-Regular")];
    let encoded: Vec<Vec<u8>> = strings.iter().map(|(_, s)| s.encode_utf16().flat_map(|u| u.to_be_bytes()).collect()).collect();
    put_u16(&mut name, 0);
    put_u16(&mut name, strings.len() as u16);
    put_u16(&mut name, 6 + 12 * strings.len() as u16);
    let mut offset = 0u16;
    for ((id, _), bytes) in strings.iter().zip(&encoded) {
        for v in [3u16, 1, 0x409, *id, bytes.len() as u16, offset] {
            put_u16(&mut name, v);
        }
        offset += bytes.len() as u16;
    }
    for bytes in &encoded {
        name.extend(bytes);
    }

    let mut os2 = Vec::new();
    put_u16(&mut os2, 3);
    put_i16(&mut os2, 560);
    put_u16(&mut os2, 400);
    put_u16(&mut os2, 5);
    put_u16(&mut os2, 0);
    for v in [650i16, 600, 0, 75, 650, 600, 0, 350, 50, 260, 0] {
        put_i16(&mut os2, v);
    }
    os2.extend([0u8; 10]); // panose
    put_u32(&mut os2, 1); // basic latin
    os2.extend([0u8; 12]);
    os2.extend(b"OPNX");
    put_u16(&mut os2, 0x40);
    put_u16(&mut os2, map.first().map_or(32, |m| m.0));
    put_u16(&mut os2, map.last().map_or(126, |m| m.0));
    put_i16(&mut os2, 800);
    put_i16(&mut os2, -200);
    put_i16(&mut os2, 200);
    put_u16(&mut os2, 900);
    put_u16(&mut os2, 250);
    put_u32(&mut os2, 1);
    put_u32(&mut os2, 0);
    put_i16(&mut os2, 700);
    put_i16(&mut os2, 700);
    put_u16(&mut os2, 0);
    put_u16(&mut os2, 32);
    put_u16(&mut os2, 0);

    let mut post = Vec::new();
    put_u32(&mut post, 0x0003_0000);
    put_u32(&mut post, 0);
    put_i16(&mut post, -100);
    put_i16(&mut post, 50);
    post.extend([0u8; 20]);

    let mut tables: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"OS/2", os2),
        (b"cmap", cmap),
        (b"glyf", glyf),
        (b"head", head),
        (b"hhea", hhea),
        (b"hmtx", hmtx),
        (b"loca", loca_bytes),
        (b"maxp", maxp),
        (b"name", name),
        (b"post", post),
    ];
    tables.sort_by(|a, b| a.0.cmp(b.0));
    let n = tables.len() as u16;
    let mut out = Vec::new();
    put_u32(&mut out, 0x0001_0000);
    put_u16(&mut out, n);
    let search = 16 * (1u16 << (15 - n.leading_zeros() as u16));
    put_u16(&mut out, search);
    put_u16(&mut out, (search / 16).trailing_zeros() as u16);
    put_u16(&mut out, n * 16 - search);
    let mut offset = 12 + 16 * n as u32;
    let mut head_at = 0;
    for (tag, data) in &tables {
        out.extend(*tag);
        put_u32(&mut out, checksum(data));
        put_u32(&mut out, offset);
        put_u32(&mut out, data.len() as u32);
        if *tag == b"head" {
            head_at = offset as usize;
        }
        offset += (data.len() as u32).div_ceil(4) * 4;
    }
    for (_, data) in &tables {
        out.extend(data);
        while out.len() % 4 != 0 {
            out.push(0);
        }
    }
    let adjust = 0xB1B0_AFBAu32.wrapping_sub(checksum(&out));
    out[head_at + 8..head_at + 12].copy_from_slice(&adjust.to_be_bytes());
    out
}

fn checksum(data: &[u8]) -> u32 {
    data.chunks(4).fold(0u32, |sum, chunk| {
        let mut word = [0u8; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        sum.wrapping_add(u32::from_be_bytes(word))
    })
}

fn put_u16(out: &mut Vec<u8>, v: u16) {
    out.extend(v.to_be_bytes());
}

fn put_i16(out: &mut Vec<u8>, v: i16) {
    out.extend(v.to_be_bytes());
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend(v.to_be_bytes());
}

pub struct TypefacePlugin;

impl Plugin for TypefacePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, install);
    }
}

/// Puts the typeface where Bevy looks for its default font.
fn install(mut fonts: ResMut<Assets<Font>>) {
    if let Ok(font) = Font::try_from_bytes(build()) {
        let _ = fonts.insert(Handle::<Font>::default().id(), font);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_typeface_is_a_valid_font() {
        let bytes = build();
        assert!(bytes.len() > 2000);
        assert!(Font::try_from_bytes(bytes).is_ok());
        // Every glyph is the same height on its grid.
        for (ch, rows) in GLYPHS {
            assert!(rows.len() == 7 || rows.len() == 8, "{ch} has {} rows", rows.len());
            let width = rows[0].len();
            assert!(rows.iter().all(|r| r.len() == width), "{ch} has ragged rows");
        }
    }
}


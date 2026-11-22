//! Text measurement for the glasses' faces (ASCII advances from the firmware,
//! CJK at full width). Rich documents use the same glyph metrics in visual_page.
#![allow(dead_code)]
const F24: i32 = 0;
const F32: i32 = 2;

const F40: i32 = 3;
fn advances(font: i32) -> (&'static [u8; 95], i32) {
    match font { F32 | F40 => (&ADV32, if font == F40 { 40 } else { 32 }), _ => (&ADV24, 24) }
}

/// Text width with one table lookup per byte.
pub fn width(font: i32, s: &[u8]) -> i32 {
    let (table, px) = advances(font);
    let mut w = 0;
    for b in s {
        let b = *b;
        if b >= 32 && b < 127 { w += table[(b - 32) as usize] as i32; }
        else if b >= 0xc0 { w += px; }
    }
    w
}

/// UTF-8 scalar at `i` and its byte length; a malformed byte stands for itself.
pub fn scalar(src: &[u8], i: usize) -> (u32, usize) {
    let b = src[i];
    let n = match b { 0xc0..=0xdf => 2, 0xe0..=0xef => 3, 0xf0..=0xf7 => 4, _ => 1 };
    if n == 1 || i + n > src.len() { return (b as u32, 1); }
    let mut c = (b & (0x7f >> n)) as u32;
    for k in 1..n { c = (c << 6) | (src[i + k] & 0x3f) as u32; }
    (c, n)
}

/// Chinese and Japanese set no spaces between words, so lines break between these characters.
pub fn cjk(c: u32) -> bool {
    matches!(c, 0x3000..=0x30ff | 0x31f0..=0x31ff | 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xf900..=0xfaff | 0xff00..=0xffef)
}

/// Closing punctuation never starts a line.
pub fn closes(c: u32) -> bool {
    matches!(c, 0x21 | 0x29 | 0x2c | 0x2e | 0x3a | 0x3b | 0x3f | 0x3001 | 0x3002 | 0x3009 | 0x300b | 0x300d
        | 0x300f | 0x3011 | 0x3015 | 0x30fb | 0x30fc | 0xff01 | 0xff09 | 0xff0c | 0xff0e | 0xff1a | 0xff1b | 0xff1f)
}

/// Opening brackets never end a line.
pub fn opens(c: u32) -> bool {
    matches!(c, 0x28 | 0x3008 | 0x300a | 0x300c | 0x300e | 0x3010 | 0x3014 | 0xff08)
}

/// Wrap a short status at word boundaries, splitting oversized words without
/// splitting UTF-8 characters. Keep explicit line breaks from the tool.
pub fn wrap_status(src: &[u8], max_w: i32, out: &mut [u8]) -> usize {
    wrap(F24, src, max_w, out)
}

pub fn wrap(font: i32, src: &[u8], max_w: i32, out: &mut [u8]) -> usize {
    let Ok(text) = core::str::from_utf8(src) else { return 0; };
    let mut len = 0;
    for (index, line) in text.split('\n').enumerate() {
        if index > 0 { if len == out.len() { return len; } out[len] = b'\n'; len += 1; }
        let mut line_w = 0;
        for word in line.split_whitespace() {
            if line_w > 0 {
                if len == out.len() { return len; }
                if line_w + width(font, b" ") + width(font, word.as_bytes()) > max_w {
                    out[len] = b'\n'; line_w = 0;
                } else { out[len] = b' '; line_w += width(font, b" "); }
                len += 1;
            }
            for ch in word.chars() {
                let mut bytes = [0; 4];
                let bytes = ch.encode_utf8(&mut bytes).as_bytes();
                let w = width(font, bytes);
                if line_w > 0 && line_w + w > max_w {
                    if len == out.len() { return len; }
                    out[len] = b'\n'; len += 1; line_w = 0;
                }
                if len + bytes.len() > out.len() { return len; }
                out[len..len + bytes.len()].copy_from_slice(bytes);
                len += bytes.len(); line_w += w;
            }
        }
    }
    len
}


const ADV24: [u8; 95] = [
    6, 7, 8, 15, 13, 19, 17, 4, 8, 8, 12, 14, 5, 7, 5, 8, 15, 10, 13, 12, 15, 14, 14, 13, 14, 14,
    7, 7, 14, 14, 14, 12, 24, 16, 15, 15, 17, 14, 13, 17, 18, 6, 11, 15, 13, 21, 18, 18, 15, 18,
    16, 14, 14, 18, 16, 24, 15, 14, 15, 8, 8, 8, 14, 8, 6, 13, 15, 12, 15, 13, 9, 14, 13, 5, 6,
    12, 4, 22, 14, 14, 15, 14, 9, 11, 10, 14, 13, 20, 12, 13, 12, 8, 6, 8, 14,
];

const ADV32: [u8; 95] = [
    8, 8, 10, 19, 18, 25, 22, 6, 10, 10, 17, 18, 7, 11, 7, 10, 19, 12, 17, 17, 19, 18, 19, 18, 20,
    18, 7, 7, 18, 18, 18, 15, 31, 22, 20, 20, 22, 17, 17, 22, 23, 7, 15, 20, 16, 27, 23, 23, 19,
    23, 20, 18, 18, 23, 21, 32, 20, 20, 20, 11, 10, 10, 18, 10, 8, 18, 19, 16, 19, 18, 11, 19, 19,
    7, 7, 16, 7, 29, 19, 18, 19, 19, 11, 14, 12, 19, 17, 27, 16, 17, 16, 10, 8, 11, 18,
];

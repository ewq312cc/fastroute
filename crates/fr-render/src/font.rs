//! A built-in 5x7 bitmap font, so labels need no font file and render the same everywhere.
//!
//! Upper case letters, digits and common punctuation. Lower case letters are drawn as upper
//! case; every character that is not in the table (e.g. non-ASCII net names) becomes an empty
//! box. The exact names always stay available in the JSON next to the image.

use tiny_skia::{Path, PathBuilder, Rect};

pub const GLYPH_W: u32 = 5;
pub const GLYPH_H: u32 = 7;
/// Horizontal advance of one character, in glyph pixels.
pub const ADVANCE: u32 = GLYPH_W + 1;

const BOX: [u8; 7] = [0b11111, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11111];

/// The 7 rows (top first) of `c`, bit 4 = leftmost column.
fn glyph(c: char) -> [u8; 7] {
    let c = c.to_ascii_uppercase();
    match c {
        ' ' => [0; 7],
        '!' => [0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0, 0b00100],
        '"' => [0b01010, 0b01010, 0b01010, 0, 0, 0, 0],
        '#' => [0b01010, 0b01010, 0b11111, 0b01010, 0b11111, 0b01010, 0b01010],
        '$' => [0b00100, 0b01111, 0b10100, 0b01110, 0b00101, 0b11110, 0b00100],
        '%' => [0b11000, 0b11001, 0b00010, 0b00100, 0b01000, 0b10011, 0b00011],
        '&' => [0b01100, 0b10010, 0b10100, 0b01000, 0b10101, 0b10010, 0b01101],
        '\'' => [0b01100, 0b00100, 0b01000, 0, 0, 0, 0],
        '(' | '{' => [0b00010, 0b00100, 0b01000, 0b01000, 0b01000, 0b00100, 0b00010],
        ')' | '}' => [0b01000, 0b00100, 0b00010, 0b00010, 0b00010, 0b00100, 0b01000],
        '*' => [0, 0b00100, 0b10101, 0b01110, 0b10101, 0b00100, 0],
        '+' => [0, 0b00100, 0b00100, 0b11111, 0b00100, 0b00100, 0],
        ',' => [0, 0, 0, 0, 0b01100, 0b00100, 0b01000],
        '-' => [0, 0, 0, 0b11111, 0, 0, 0],
        '.' => [0, 0, 0, 0, 0, 0b01100, 0b01100],
        '/' => [0, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0],
        '0' => [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110],
        '1' => [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        '2' => [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111],
        '3' => [0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110],
        '4' => [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
        '5' => [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
        '6' => [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110],
        '7' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
        '8' => [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
        '9' => [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100],
        ':' => [0, 0b01100, 0b01100, 0, 0b01100, 0b01100, 0],
        ';' => [0, 0b01100, 0b01100, 0, 0b01100, 0b00100, 0b01000],
        '<' => [0b00010, 0b00100, 0b01000, 0b10000, 0b01000, 0b00100, 0b00010],
        '=' => [0, 0, 0b11111, 0, 0b11111, 0, 0],
        '>' => [0b01000, 0b00100, 0b00010, 0b00001, 0b00010, 0b00100, 0b01000],
        '?' => [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0, 0b00100],
        '@' => [0b01110, 0b10001, 0b00001, 0b01101, 0b10101, 0b10101, 0b01110],
        'A' => [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'B' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110],
        'C' => [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110],
        'D' => [0b11100, 0b10010, 0b10001, 0b10001, 0b10001, 0b10010, 0b11100],
        'E' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111],
        'F' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000],
        'G' => [0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111],
        'H' => [0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'I' => [0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        'J' => [0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100],
        'K' => [0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001],
        'L' => [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111],
        'M' => [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001],
        'N' => [0b10001, 0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001],
        'O' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'P' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000],
        'Q' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101],
        'R' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001],
        'S' => [0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110],
        'T' => [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100],
        'U' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'V' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100],
        'W' => [0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b10101, 0b01010],
        'X' => [0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001],
        'Y' => [0b10001, 0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100],
        'Z' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111],
        '[' => [0b01110, 0b01000, 0b01000, 0b01000, 0b01000, 0b01000, 0b01110],
        '\\' => [0, 0b10000, 0b01000, 0b00100, 0b00010, 0b00001, 0],
        ']' => [0b01110, 0b00010, 0b00010, 0b00010, 0b00010, 0b00010, 0b01110],
        '^' => [0b00100, 0b01010, 0b10001, 0, 0, 0, 0],
        '_' => [0, 0, 0, 0, 0, 0, 0b11111],
        '|' => [0b00100; 7],
        '~' => [0, 0, 0b01000, 0b10101, 0b00010, 0, 0],
        _ => BOX,
    }
}

/// Width of `text` in glyph pixels (without the trailing gap).
pub fn text_width(text: &str) -> u32 {
    let n = text.chars().count() as u32;
    if n == 0 {
        0
    } else {
        n * ADVANCE - 1
    }
}

/// The pixels of `text` as one path of unit squares, scaled by `scale` and with its top-left
/// corner at `(x, y)`; `None` for an empty text.
pub fn text_path(text: &str, x: f32, y: f32, scale: u32) -> Option<Path> {
    let s = scale.max(1) as f32;
    let mut pb = PathBuilder::new();
    let mut any = false;
    for (i, ch) in text.chars().enumerate() {
        let gx = x + (i as u32 * ADVANCE) as f32 * s;
        for (row, bits) in glyph(ch).iter().enumerate() {
            let mut col = 0;
            while col < GLYPH_W {
                // merge a run of lit pixels into one rectangle
                if bits & (1 << (GLYPH_W - 1 - col)) != 0 {
                    let start = col;
                    while col < GLYPH_W && bits & (1 << (GLYPH_W - 1 - col)) != 0 {
                        col += 1;
                    }
                    if let Some(r) = Rect::from_xywh(gx + start as f32 * s, y + row as f32 * s, (col - start) as f32 * s, s) {
                        pb.push_rect(r);
                        any = true;
                    }
                } else {
                    col += 1;
                }
            }
        }
    }
    if any {
        pb.finish()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths() {
        assert_eq!(text_width(""), 0);
        assert_eq!(text_width("A"), 5);
        assert_eq!(text_width("AB"), 11);
    }

    #[test]
    fn every_printable_ascii_char_draws_something_except_space() {
        for c in (0x21u8..0x7f).map(|b| b as char) {
            assert!(text_path(&c.to_string(), 0.0, 0.0, 1).is_some(), "{c:?} has no glyph");
        }
        assert!(text_path(" ", 0.0, 0.0, 1).is_none());
        assert!(text_path("", 0.0, 0.0, 1).is_none());
    }

    #[test]
    fn lower_case_is_upper_case_and_unknown_is_a_box() {
        assert_eq!(glyph('a'), glyph('A'));
        assert_eq!(glyph('中'), BOX);
        assert_ne!(glyph('O'), glyph('0'), "O and 0 must differ");
        assert_ne!(glyph('I'), glyph('1'));
    }

    #[test]
    fn glyphs_fit_five_columns() {
        for b in 0x20u8..0x7f {
            for row in glyph(b as char) {
                assert!(row < 32, "{:?}", b as char);
            }
        }
    }
}

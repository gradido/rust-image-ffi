//! Which quality a JPEG was written with -- as near as its quantization tables say.
//!
//! A JPEG does not store a quality. It stores the tables its coefficients were divided by, and
//! the encoders of the libjpeg family make those by scaling the two tables of the JPEG standard
//! (Annex K) with the quality: `scale = 5000 / q` below 50, `200 - 2 q` from there on, each entry
//! `(standard * scale + 50) / 100`, held between 1 and 255. This module runs that forwards for
//! every quality from 1 to 100 and answers the one whose tables are nearest the file's.
//!
//! For a file from such an encoder the answer is exact. For one with tables of its own -- some
//! cameras, Photoshop, mozjpeg at its own defaults -- it is the quality whose standard tables
//! divide about as coarsely, which is what a caller that wants to encode "as well as this, and
//! no better" needs.
//!
//! Which table is the luminance's is said by the frame header, not by the table's number: nearly
//! every encoder numbers them 0 and 1, and a file need not. It reads nothing but the table
//! segments and the frame header, and everything through checked slices: what it is handed is the
//! sender's bytes.

/// The standard's luminance and chrominance tables, in reading order.
const STANDARD: [[u16; 64]; 2] = [
    [
        16, 11, 10, 16, 24, 40, 51, 61, 12, 12, 14, 19, 26, 58, 60, 55, 14, 13, 16, 24, 40, 57, 69, 56, 14,
        17, 22, 29, 51, 87, 80, 62, 18, 22, 37, 56, 68, 109, 103, 77, 24, 35, 55, 64, 81, 104, 113, 92, 49,
        64, 78, 87, 103, 121, 120, 101, 72, 92, 95, 98, 112, 100, 103, 99,
    ],
    [
        17, 18, 24, 47, 99, 99, 99, 99, 18, 21, 26, 66, 99, 99, 99, 99, 24, 26, 56, 99, 99, 99, 99, 99, 47,
        66, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
        99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
    ],
];

/// A file stores a table along the zigzag; this is where each stored entry sits in reading order.
const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7,
    14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39,
    46, 53, 60, 61, 54, 47, 55, 62, 63,
];

/// What stands in front of the first scan: the tables by their number, in reading order, and
/// which of them the frame header gives the first component -- the luminance -- and the second.
#[derive(Default)]
struct Header {
    tables: [Option<[u16; 64]>; 4],
    selectors: Option<(u8, Option<u8>)>,
}

/// Walks the segments in front of the first scan.
fn header(jpeg: &[u8]) -> Header {
    let mut found = Header::default();
    if jpeg.get(..2) != Some(&[0xff, 0xd8]) {
        return found;
    }
    let mut at = 2;
    loop {
        // A marker is ff and a byte that is neither 00 nor ff; ff may be repeated before it.
        let Some(&[0xff, marker]) = jpeg.get(at..at + 2) else {
            return found;
        };
        at += 1;
        match marker {
            0xff => continue,
            // Without a length: a restart marker, or TEM.
            0x01 | 0xd0..=0xd7 => {
                at += 1;
                continue;
            }
            // The pixels begin, or the file ends: what counts stands before this.
            0xda | 0xd9 | 0x00 => return found,
            _ => {}
        }
        let Some(&[high, low]) = jpeg.get(at + 1..at + 3) else {
            return found;
        };
        let length = u16::from_be_bytes([high, low]) as usize;
        let Some(mut body) = jpeg.get(at + 3..at + 1 + length) else {
            return found;
        };
        at += 1 + length;
        match marker {
            // The quantization tables. One segment may define several: a byte of precision and
            // number, then 64 entries of one byte, or of two when the precision says so.
            0xdb => {
                while let Some((&head, rest)) = body.split_first() {
                    let wide = head >> 4 != 0;
                    let Some(entries) = rest.get(..if wide { 128 } else { 64 }) else {
                        return found;
                    };
                    body = &rest[entries.len()..];
                    let mut table = [0u16; 64];
                    for (k, &natural) in ZIGZAG.iter().enumerate() {
                        table[natural] = if wide {
                            u16::from_be_bytes([entries[2 * k], entries[2 * k + 1]])
                        } else {
                            entries[k] as u16
                        };
                    }
                    if let Some(slot) = found.tables.get_mut((head & 0x0f) as usize) {
                        *slot = Some(table);
                    }
                }
            }
            // The frame header, in any of its kinds -- c4, c8 and cc are other segments. Six
            // bytes of precision and size, then per component its id, its sampling factors and
            // the number of its quantization table.
            0xc0..=0xcf if !matches!(marker, 0xc4 | 0xc8 | 0xcc) => {
                let selector = |component: usize| body.get(6 + 3 * component + 2).copied();
                let components = body.get(5).copied().unwrap_or(0);
                if let Some(luminance) = selector(0) {
                    let chrominance = if components >= 2 { selector(1) } else { None };
                    found.selectors = Some((luminance, chrominance));
                }
            }
            _ => {}
        }
    }
}

/// What libjpeg makes of a standard table at a quality.
fn scaled(standard: &[u16; 64], quality: u32) -> [u16; 64] {
    let scale = if quality < 50 {
        5000 / quality
    } else {
        200 - 2 * quality
    };
    standard.map(|entry| ((entry as u32 * scale + 50) / 100).clamp(1, 255) as u16)
}

/// 1..=100, or `None` for what is no JPEG or carries no luminance table before its first scan.
pub fn estimate(jpeg: &[u8]) -> Option<u8> {
    let found = header(jpeg);
    // Without a frame header in front of the first scan, the numbers every encoder uses.
    let (luminance, chrominance) = found.selectors.unwrap_or((0, Some(1)));
    let table = |selector: u8| found.tables.get(selector as usize).and_then(Option::as_ref);
    let luminance_table = table(luminance)?;
    // A file may divide every component by the one table. That table is then measured as the
    // luminance's, once, and not held against the chrominance's standard as well.
    let chrominance_table = chrominance.filter(|&c| c != luminance).and_then(table);
    let distance = |quality: u32| -> u64 {
        [
            (Some(luminance_table), &STANDARD[0]),
            (chrominance_table, &STANDARD[1]),
        ]
        .into_iter()
        .filter_map(|(table, standard)| table.map(|table| (table, scaled(standard, quality))))
        .flat_map(|(table, candidate)| table.iter().zip(candidate).map(|(&a, b)| a.abs_diff(b) as u64))
        .sum()
    };
    // The highest of equally near qualities: at the low end several give the same tables, and
    // the answer is used as a bound from above.
    (1..=100u32)
        .rev()
        .min_by_key(|&quality| distance(quality))
        .map(|quality| quality as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file with nothing in it but the tables libjpeg would write at `quality`.
    fn with_tables(quality: u32, wide: bool) -> Vec<u8> {
        let mut jpeg = vec![0xff, 0xd8];
        for (id, standard) in STANDARD.iter().enumerate() {
            let table = scaled(standard, quality);
            jpeg.extend([0xff, 0xdb]);
            jpeg.extend(((if wide { 131 } else { 67 }) as u16).to_be_bytes());
            jpeg.push(((wide as u8) << 4) | id as u8);
            for natural in ZIGZAG {
                if wide {
                    jpeg.extend(table[natural].to_be_bytes());
                } else {
                    jpeg.push(table[natural] as u8);
                }
            }
        }
        jpeg.extend([0xff, 0xda, 0, 2]);
        jpeg
    }

    /// The same, with the tables numbered as the caller says and a frame header that names them:
    /// `luminance` and `chrominance` are table numbers, `components` how many the frame has.
    fn with_frame(quality: u32, luminance: u8, chrominance: u8, components: u8) -> Vec<u8> {
        let mut jpeg = vec![0xff, 0xd8];
        let mut numbers = vec![luminance];
        if chrominance != luminance {
            numbers.push(chrominance);
        }
        for (number, standard) in numbers.iter().zip(&STANDARD) {
            jpeg.extend([0xff, 0xdb, 0, 67, *number]);
            let table = scaled(standard, quality);
            jpeg.extend(ZIGZAG.map(|natural| table[natural] as u8));
        }
        jpeg.extend([0xff, 0xc0]);
        jpeg.extend((8 + 3 * components as u16).to_be_bytes());
        jpeg.extend([8, 0, 16, 0, 16, components]);
        for component in 0..components {
            let selector = if component == 0 { luminance } else { chrominance };
            jpeg.extend([component + 1, 0x11, selector]);
        }
        jpeg.extend([0xff, 0xda, 0, 2]);
        jpeg
    }

    #[test]
    fn the_frame_header_says_which_table_is_the_luminances() {
        for quality in [20, 40, 60, 75, 90, 100] {
            let expected = Some(quality as u8);
            // As every encoder numbers them, swapped, and with the two numbers nobody uses.
            assert_eq!(estimate(&with_frame(quality, 0, 1, 3)), expected);
            assert_eq!(estimate(&with_frame(quality, 1, 0, 3)), expected);
            assert_eq!(estimate(&with_frame(quality, 2, 3, 3)), expected);
            assert_eq!(estimate(&with_frame(quality, 3, 0, 3)), expected);
            // A gray picture, whose one table has any number.
            assert_eq!(estimate(&with_frame(quality, 0, 0, 1)), expected);
            assert_eq!(estimate(&with_frame(quality, 1, 1, 1)), expected);
            // Three components and one table for all of them.
            assert_eq!(estimate(&with_frame(quality, 0, 0, 3)), expected);
        }
    }

    #[test]
    fn a_frame_that_names_a_table_the_file_does_not_have_has_no_quality() {
        let mut jpeg = with_frame(60, 0, 1, 3);
        let frame = jpeg.windows(2).position(|w| w == [0xff, 0xc0]).unwrap();
        // The luminance's selector: two bytes of marker, two of length, six of header, then the
        // first component's id and sampling.
        jpeg[frame + 12] = 2;
        assert_eq!(estimate(&jpeg), None);
        // A number that is no table at all.
        jpeg[frame + 12] = 200;
        assert_eq!(estimate(&jpeg), None);
        // Only the chrominance's is missing: the luminance's table still says it.
        let mut jpeg = with_frame(60, 0, 1, 3);
        jpeg[frame + 15] = 3;
        assert_eq!(estimate(&jpeg), Some(60));
    }

    #[test]
    fn every_quality_is_found_again() {
        // Below 5 the tables are all 255 whatever the quality; from there on they differ.
        for quality in 5..=100 {
            assert_eq!(estimate(&with_tables(quality, false)), Some(quality as u8));
            assert_eq!(estimate(&with_tables(quality, true)), Some(quality as u8));
        }
    }

    #[test]
    fn tables_that_are_nobodys_scaling_get_the_nearest_quality() {
        // Every entry one coarser than quality 60's: still nearer to 60 than to 59 or 61?
        let mut jpeg = with_tables(60, false);
        for entry in &mut jpeg[7..7 + 64] {
            *entry += 1;
        }
        let found = estimate(&jpeg).unwrap();
        assert!((57..=60).contains(&found), "{found}");
    }

    #[test]
    fn what_carries_no_table_has_no_quality() {
        assert_eq!(estimate(b""), None);
        assert_eq!(estimate(b"\x89PNG\r\n\x1a\n"), None);
        assert_eq!(estimate(&[0xff, 0xd8, 0xff, 0xda, 0, 2]), None);
        // Only a chrominance table.
        let jpeg = with_tables(60, false);
        let mut chroma_only = vec![0xff, 0xd8];
        chroma_only.extend(&jpeg[2 + 69..]);
        assert_eq!(estimate(&chroma_only), None);
    }

    #[test]
    fn a_file_cut_anywhere_is_read_without_a_panic() {
        for jpeg in [with_tables(75, true), with_frame(75, 1, 0, 3)] {
            for end in 0..jpeg.len() {
                let _ = estimate(&jpeg[..end]);
            }
        }
        // A frame header that claims more components than it has bytes for, and an empty one.
        let _ = estimate(&[0xff, 0xd8, 0xff, 0xc0, 0, 8, 8, 0, 16, 0, 16, 255]);
        let _ = estimate(&[0xff, 0xd8, 0xff, 0xc2, 0, 2]);
        // Lengths that point outside the file, and a table id that is no table.
        let _ = estimate(&[0xff, 0xd8, 0xff, 0xdb, 0xff, 0xff, 0x00, 1, 2, 3]);
        let _ = estimate(&[0xff, 0xd8, 0xff, 0xdb, 0x00, 0x00]);
        let _ = estimate(&[0xff, 0xd8, 0xff, 0xdb, 0x00, 0x01]);
        let mut odd = with_tables(75, false);
        odd[6] = 0x0f;
        let _ = estimate(&odd);
        let _ = estimate(&[0xff, 0xd8, 0xff, 0xff, 0xff, 0xff]);
    }
}

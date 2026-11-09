//! Compiles the built-in images into a GResource bundle for avizo-service.
//!
//! The PNG icons are stored as uncompressed GdkPixdata, so that GdkPixbuf
//! can read them from the bundle without any image loader at run time (the
//! same as `preprocess="to-pixdata"`). The PNGs are decoded here, without a
//! loader either, so the build does not depend on the host's gdk-pixbuf
//! loaders. The SVG sources are bundled as they are, for scaled outputs.

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

/// The built-in image resources, in the order `--list-resources` prints them.
const RESOURCES: [&str; 18] = [
    "volume_muted",
    "volume_muted_dark",
    "volume_low",
    "volume_low_dark",
    "volume_medium",
    "volume_medium_dark",
    "volume_high",
    "volume_high_dark",
    "mic_muted",
    "mic_muted_dark",
    "mic_unmuted",
    "mic_unmuted_dark",
    "brightness_low",
    "brightness_low_dark",
    "brightness_medium",
    "brightness_medium_dark",
    "brightness_high",
    "brightness_high_dark",
];

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let images = manifest.join("data").join("images");
    let pixdata_dir = out.join("pixdata");
    fs::create_dir_all(&pixdata_dir).expect("create pixdata directory");

    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<gresources>\n  <gresource prefix=\"/org/danb/avizo\">\n",
    );
    for name in RESOURCES {
        let png = images.join(format!("{name}.png"));
        let svg = images.join(format!("{name}.svg"));
        println!("cargo:rerun-if-changed={}", png.display());
        println!("cargo:rerun-if-changed={}", svg.display());

        let data = fs::read(&png).unwrap_or_else(|e| panic!("{}: {e}", png.display()));
        let image = png::decode(&data).unwrap_or_else(|e| panic!("{}: {e}", png.display()));
        let pixdata = pixdata_dir.join(format!("{name}.pixdata"));
        fs::write(&pixdata, image.to_pixdata()).expect("write pixdata");

        let _ = writeln!(
            xml,
            "    <file alias=\"data/images/{name}.png\">{name}.pixdata</file>"
        );
        let _ = writeln!(
            xml,
            "    <file alias=\"data/images/{name}.svg\">{name}.svg</file>"
        );
    }
    xml.push_str("  </gresource>\n</gresources>\n");

    let xml_path = out.join("avizo.gresource.xml");
    fs::write(&xml_path, xml).expect("write gresource xml");
    glib_build_tools::compile_resources(
        &[pixdata_dir.as_path(), images.as_path()],
        xml_path.to_str().expect("utf-8 OUT_DIR"),
        "avizo.gresource",
    );
    println!("cargo:rerun-if-changed=build.rs");
}

/// A minimal PNG decoder (all colour types, bit depths 1-16, no interlace
/// restriction) that yields 8-bit RGB or RGBA.
mod png {
    pub struct Image {
        pub width: u32,
        pub height: u32,
        pub has_alpha: bool,
        pub pixels: Vec<u8>,
    }

    impl Image {
        /// Serialises the image as a raw GdkPixdata stream (big endian).
        pub fn to_pixdata(&self) -> Vec<u8> {
            let channels = if self.has_alpha { 4 } else { 3 };
            let rowstride = self.width * channels;
            let mut out = Vec::with_capacity(24 + self.pixels.len());
            let color_type: u32 = if self.has_alpha { 0x02 } else { 0x01 };
            let pixdata_type = color_type | 0x0001_0000 | 0x0100_0000;
            out.extend_from_slice(&0x4764_6b50u32.to_be_bytes());
            out.extend_from_slice(&(24 + self.pixels.len() as u32).to_be_bytes());
            out.extend_from_slice(&pixdata_type.to_be_bytes());
            out.extend_from_slice(&rowstride.to_be_bytes());
            out.extend_from_slice(&self.width.to_be_bytes());
            out.extend_from_slice(&self.height.to_be_bytes());
            out.extend_from_slice(&self.pixels);
            out
        }
    }

    fn be32(data: &[u8], at: usize) -> Result<u32, String> {
        data.get(at..at + 4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
            .ok_or_else(|| "truncated PNG".to_string())
    }

    pub fn decode(data: &[u8]) -> Result<Image, String> {
        if data.len() < 8 || &data[..8] != b"\x89PNG\r\n\x1a\n" {
            return Err("not a PNG file".into());
        }
        let mut at = 8;
        let mut header = None;
        let mut palette: Vec<u8> = Vec::new();
        let mut trns: Vec<u8> = Vec::new();
        let mut idat = Vec::new();
        while at + 8 <= data.len() {
            let length = be32(data, at)? as usize;
            let kind = &data[at + 4..at + 8];
            let body = data
                .get(at + 8..at + 8 + length)
                .ok_or_else(|| "truncated PNG chunk".to_string())?;
            match kind {
                b"IHDR" => {
                    if body.len() < 13 {
                        return Err("bad IHDR".into());
                    }
                    header = Some((be32(body, 0)?, be32(body, 4)?, body[8], body[9], body[12]));
                }
                b"PLTE" => palette = body.to_vec(),
                b"tRNS" => trns = body.to_vec(),
                b"IDAT" => idat.extend_from_slice(body),
                b"IEND" => break,
                _ => {}
            }
            at += 12 + length;
        }
        let (width, height, depth, color, interlace) =
            header.ok_or_else(|| "missing IHDR".to_string())?;
        let samples = match color {
            0 => 1,
            2 => 3,
            3 => 1,
            4 => 2,
            6 => 4,
            _ => return Err(format!("unsupported colour type {color}")),
        };
        let raw = inflate::zlib(&idat)?;
        let bits_per_pixel = samples * depth as usize;
        let bpp = bits_per_pixel.div_ceil(8);

        // Collect each pass (or the whole image) as unfiltered rows.
        let passes: Vec<(usize, usize, usize, usize)> = if interlace == 1 {
            vec![
                (0, 0, 8, 8),
                (4, 0, 8, 8),
                (0, 4, 4, 8),
                (2, 0, 4, 4),
                (0, 2, 2, 4),
                (1, 0, 2, 2),
                (0, 1, 1, 2),
            ]
        } else {
            vec![(0, 0, 1, 1)]
        };
        let (w, h) = (width as usize, height as usize);
        let mut samples_out = vec![0u16; w * h * samples];
        let mut pos = 0;
        for (x0, y0, dx, dy) in passes {
            if x0 >= w || y0 >= h {
                continue;
            }
            let pw = (w - x0).div_ceil(dx);
            let ph = (h - y0).div_ceil(dy);
            let stride = (pw * bits_per_pixel).div_ceil(8);
            let mut previous = vec![0u8; stride];
            for row in 0..ph {
                let filter = *raw.get(pos).ok_or("truncated image data")?;
                let mut line = raw
                    .get(pos + 1..pos + 1 + stride)
                    .ok_or("truncated image data")?
                    .to_vec();
                pos += 1 + stride;
                unfilter(filter, &mut line, &previous, bpp)?;
                for col in 0..pw {
                    for s in 0..samples {
                        let index = col * samples + s;
                        let value = match depth {
                            16 => u16::from_be_bytes([line[index * 2], line[index * 2 + 1]]),
                            8 => line[index] as u16,
                            d => {
                                let bit = index * d as usize;
                                let byte = line[bit / 8];
                                let shift = 8 - d as usize - bit % 8;
                                ((byte >> shift) & ((1u8 << d) - 1)) as u16
                            }
                        };
                        let x = x0 + col * dx;
                        let y = y0 + row * dy;
                        samples_out[(y * w + x) * samples + s] = value;
                    }
                }
                previous = line;
            }
        }

        let to8 = |v: u16| -> u8 {
            match depth {
                16 => ((v as u32 + 128) >> 8).min(255) as u8,
                8 => v as u8,
                d => ((v as u32 * 255) / ((1u32 << d) - 1)) as u8,
            }
        };
        let has_alpha = matches!(color, 4 | 6) || !trns.is_empty();
        let channels = if has_alpha { 4 } else { 3 };
        let mut pixels = Vec::with_capacity(w * h * channels);
        for i in 0..w * h {
            let px = &samples_out[i * samples..(i + 1) * samples];
            let (rgb, alpha) = match color {
                0 => {
                    let transparent =
                        trns.len() >= 2 && px[0] == u16::from_be_bytes([trns[0], trns[1]]);
                    let g = to8(px[0]);
                    ([g, g, g], if transparent { 0 } else { 255 })
                }
                2 => {
                    let transparent = trns.len() >= 6
                        && px[0] == u16::from_be_bytes([trns[0], trns[1]])
                        && px[1] == u16::from_be_bytes([trns[2], trns[3]])
                        && px[2] == u16::from_be_bytes([trns[4], trns[5]]);
                    (
                        [to8(px[0]), to8(px[1]), to8(px[2])],
                        if transparent { 0 } else { 255 },
                    )
                }
                3 => {
                    let index = px[0] as usize;
                    let entry = palette
                        .get(index * 3..index * 3 + 3)
                        .ok_or("palette index out of range")?;
                    (
                        [entry[0], entry[1], entry[2]],
                        trns.get(index).copied().unwrap_or(255),
                    )
                }
                4 => {
                    let g = to8(px[0]);
                    ([g, g, g], to8(px[1]))
                }
                _ => ([to8(px[0]), to8(px[1]), to8(px[2])], to8(px[3])),
            };
            pixels.extend_from_slice(&rgb);
            if has_alpha {
                pixels.push(alpha);
            }
        }
        Ok(Image {
            width,
            height,
            has_alpha,
            pixels,
        })
    }

    fn unfilter(filter: u8, line: &mut [u8], previous: &[u8], bpp: usize) -> Result<(), String> {
        for i in 0..line.len() {
            let left = if i >= bpp { line[i - bpp] } else { 0 };
            let up = previous[i];
            let up_left = if i >= bpp { previous[i - bpp] } else { 0 };
            let predictor = match filter {
                0 => 0,
                1 => left,
                2 => up,
                3 => ((left as u16 + up as u16) / 2) as u8,
                4 => {
                    let p = left as i16 + up as i16 - up_left as i16;
                    let (pa, pb, pc) = (
                        (p - left as i16).abs(),
                        (p - up as i16).abs(),
                        (p - up_left as i16).abs(),
                    );
                    if pa <= pb && pa <= pc {
                        left
                    } else if pb <= pc {
                        up
                    } else {
                        up_left
                    }
                }
                _ => return Err(format!("bad PNG filter {filter}")),
            };
            line[i] = line[i].wrapping_add(predictor);
        }
        Ok(())
    }

    /// A small DEFLATE (RFC 1951) decoder behind a zlib (RFC 1950) wrapper.
    mod inflate {
        struct Bits<'a> {
            data: &'a [u8],
            pos: usize,
            bit: u32,
            count: u32,
        }

        impl Bits<'_> {
            fn need(&mut self, n: u32) -> Result<u32, String> {
                while self.count < n {
                    let byte = *self.data.get(self.pos).ok_or("truncated deflate stream")?;
                    self.pos += 1;
                    self.bit |= (byte as u32) << self.count;
                    self.count += 8;
                }
                let value = self.bit & ((1u64 << n) - 1) as u32;
                self.bit >>= n;
                self.count -= n;
                Ok(value)
            }

            fn align(&mut self) {
                self.bit = 0;
                self.count = 0;
            }
        }

        struct Huffman {
            counts: [u16; 16],
            symbols: Vec<u16>,
        }

        impl Huffman {
            fn new(lengths: &[u8]) -> Huffman {
                let mut counts = [0u16; 16];
                for &l in lengths {
                    counts[l as usize] += 1;
                }
                counts[0] = 0;
                let mut offsets = [0u16; 16];
                for i in 1..16 {
                    offsets[i] = offsets[i - 1] + counts[i - 1];
                }
                let mut symbols = vec![0u16; lengths.len()];
                for (symbol, &l) in lengths.iter().enumerate() {
                    if l != 0 {
                        symbols[offsets[l as usize] as usize] = symbol as u16;
                        offsets[l as usize] += 1;
                    }
                }
                Huffman { counts, symbols }
            }

            fn decode(&self, bits: &mut Bits) -> Result<u16, String> {
                let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
                for len in 1..16 {
                    code |= bits.need(1)? as i32;
                    let count = self.counts[len] as i32;
                    if code - count < first {
                        return Ok(self.symbols[(index + (code - first)) as usize]);
                    }
                    index += count;
                    first += count;
                    first <<= 1;
                    code <<= 1;
                }
                Err("bad Huffman code".into())
            }
        }

        const LEN_BASE: [u16; 29] = [
            3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99,
            115, 131, 163, 195, 227, 258,
        ];
        const LEN_EXTRA: [u8; 29] = [
            0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
        ];
        const DIST_BASE: [u16; 30] = [
            1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025,
            1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
        ];
        const DIST_EXTRA: [u8; 30] = [
            0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12,
            12, 13, 13,
        ];

        pub fn zlib(data: &[u8]) -> Result<Vec<u8>, String> {
            if data.len() < 2 || (data[0] & 0x0f) != 8 {
                return Err("not a zlib stream".into());
            }
            let mut bits = Bits {
                data: &data[2..],
                pos: 0,
                bit: 0,
                count: 0,
            };
            let mut out = Vec::new();
            loop {
                let last = bits.need(1)?;
                match bits.need(2)? {
                    0 => {
                        bits.align();
                        let p = bits.pos;
                        let header = bits.data.get(p..p + 4).ok_or("truncated stored block")?;
                        let len = u16::from_le_bytes([header[0], header[1]]) as usize;
                        let block = bits
                            .data
                            .get(p + 4..p + 4 + len)
                            .ok_or("truncated stored block")?;
                        out.extend_from_slice(block);
                        bits.pos = p + 4 + len;
                    }
                    1 => {
                        let mut lengths = [0u8; 288];
                        for (i, l) in lengths.iter_mut().enumerate() {
                            *l = match i {
                                0..=143 => 8,
                                144..=255 => 9,
                                256..=279 => 7,
                                _ => 8,
                            };
                        }
                        let lit = Huffman::new(&lengths);
                        let dist = Huffman::new(&[5u8; 30]);
                        codes(&mut bits, &mut out, &lit, &dist)?;
                    }
                    2 => {
                        let hlit = bits.need(5)? as usize + 257;
                        let hdist = bits.need(5)? as usize + 1;
                        let hclen = bits.need(4)? as usize + 4;
                        const ORDER: [usize; 19] = [
                            16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
                        ];
                        let mut code_lengths = [0u8; 19];
                        for &o in ORDER.iter().take(hclen) {
                            code_lengths[o] = bits.need(3)? as u8;
                        }
                        let code = Huffman::new(&code_lengths);
                        let mut lengths = vec![0u8; hlit + hdist];
                        let mut i = 0;
                        while i < hlit + hdist {
                            let symbol = code.decode(&mut bits)?;
                            let (value, repeat) = match symbol {
                                0..=15 => (symbol as u8, 1),
                                16 => {
                                    let prev = *lengths
                                        .get(i.wrapping_sub(1))
                                        .ok_or("repeat without previous length")?;
                                    (prev, 3 + bits.need(2)? as usize)
                                }
                                17 => (0, 3 + bits.need(3)? as usize),
                                _ => (0, 11 + bits.need(7)? as usize),
                            };
                            for _ in 0..repeat {
                                *lengths.get_mut(i).ok_or("too many code lengths")? = value;
                                i += 1;
                            }
                        }
                        let lit = Huffman::new(&lengths[..hlit]);
                        let dist = Huffman::new(&lengths[hlit..]);
                        codes(&mut bits, &mut out, &lit, &dist)?;
                    }
                    _ => return Err("bad deflate block type".into()),
                }
                if last == 1 {
                    return Ok(out);
                }
            }
        }

        fn codes(
            bits: &mut Bits,
            out: &mut Vec<u8>,
            lit: &Huffman,
            dist: &Huffman,
        ) -> Result<(), String> {
            loop {
                let symbol = lit.decode(bits)? as usize;
                match symbol {
                    0..=255 => out.push(symbol as u8),
                    256 => return Ok(()),
                    _ => {
                        let s = symbol - 257;
                        if s >= 29 {
                            return Err("bad length symbol".into());
                        }
                        let len = LEN_BASE[s] as usize + bits.need(LEN_EXTRA[s] as u32)? as usize;
                        let d = dist.decode(bits)? as usize;
                        if d >= 30 {
                            return Err("bad distance symbol".into());
                        }
                        let distance =
                            DIST_BASE[d] as usize + bits.need(DIST_EXTRA[d] as u32)? as usize;
                        if distance > out.len() {
                            return Err("distance too far back".into());
                        }
                        let start = out.len() - distance;
                        for k in 0..len {
                            let byte = out[start + k];
                            out.push(byte);
                        }
                    }
                }
            }
        }
    }
}

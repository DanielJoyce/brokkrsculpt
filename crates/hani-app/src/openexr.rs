// SPDX-License-Identifier: AGPL-3.0-only

//! An OpenEXR reader for vector displacement maps: the red, green and blue
//! of a single-part image, as floats.
//!
//! It replaced the `exr` crate on 2026-11-27 (`docs/DEPENDENCIES.md`,
//! proposal 6), which brought seven crates for this one read. It reads:
//!
//! - one part, in scanlines or in tiles (the full-resolution level of a
//!   tiled file, which is what a mipmapped or ripmapped one keeps first);
//!   not deep data, not several parts;
//! - channels in `half`, `float` or `uint`; `R`, `G` and `B` are taken, any
//!   others (`A`, `Z`) skipped, and one missing reads as nought;
//! - no compression, RLE, ZIPS or ZIP. ZIP is the OpenEXR library's own
//!   default; what Blender, Mudbox and ZBrush write for a displacement map
//!   was not checked on their files.
//!
//! PIZ, PXR24, B44 and DWA are refused by name rather than read wrongly:
//! the message says which, so a user can save the map again with ZIP. ZIP
//! data is inflated by `miniz_oxide`, which PNG already brings.
//!
//! The layout, from the OpenEXR file format document: a magic number and a
//! version, a header of named attributes ended by an empty name, a table of
//! one offset per chunk, and the chunks. A scanline chunk is its first
//! line's number, a size and that many bytes; a tile is its tile and level
//! numbers, a size and the bytes. Either holds lines of a rectangle, each
//! line its channels one after another in the header's (alphabetical)
//! order. Compressed data is first
//! split in two by byte and each half delta-coded; see [`unpredict`].

/// Why a file was not read.
pub type Refusal = String;

const MAGIC: u32 = 20_000_630;

/// The most pixels read, as a guard against a header that asks for more
/// memory than a picture of displacement needs: 16384 by 16384.
const MOST_PIXELS: u64 = 16_384 * 16_384;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sample {
    Uint,
    Half,
    Float,
}

impl Sample {
    fn bytes(self) -> usize {
        match self {
            Sample::Half => 2,
            Sample::Uint | Sample::Float => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Compression {
    None,
    Rle,
    Zips,
    Zip,
}

impl Compression {
    fn lines_per_chunk(self) -> usize {
        match self {
            Compression::None | Compression::Rle | Compression::Zips => 1,
            Compression::Zip => 16,
        }
    }
}

#[derive(Debug, Clone)]
struct Channel {
    name: String,
    sample: Sample,
    x_sampling: i32,
    y_sampling: i32,
}

/// A picture as read: its size and each pixel's red, green and blue.
#[derive(Debug, Clone, PartialEq)]
pub struct Picture {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<[f32; 3]>,
}

struct Bytes<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Bytes<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], Refusal> {
        let end = self.at.checked_add(count).filter(|end| *end <= self.bytes.len());
        let end = end.ok_or_else(|| "the file ends too soon".to_string())?;
        let taken = &self.bytes[self.at..end];
        self.at = end;
        Ok(taken)
    }
    fn u32(&mut self) -> Result<u32, Refusal> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("four bytes")))
    }
    fn i32(&mut self) -> Result<i32, Refusal> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().expect("four bytes")))
    }
    fn u64(&mut self) -> Result<u64, Refusal> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().expect("eight bytes")))
    }
    /// A string ended by a nought, of at most 255 bytes.
    fn name(&mut self) -> Result<String, Refusal> {
        let rest = &self.bytes[self.at.min(self.bytes.len())..];
        let end = rest
            .iter()
            .take(256)
            .position(|byte| *byte == 0)
            .ok_or_else(|| "a name in the header is not ended".to_string())?;
        let name = String::from_utf8_lossy(&rest[..end]).into_owned();
        self.at += end + 1;
        Ok(name)
    }
}

/// A half-precision float's bits as an `f32`.
pub fn half_to_f32(bits: u16) -> f32 {
    let sign = u32::from(bits >> 15) << 31;
    let exponent = u32::from((bits >> 10) & 0x1f);
    let mantissa = u32::from(bits & 0x3ff);
    let value = match (exponent, mantissa) {
        (0, 0) => sign,
        // Subnormal: scaled up into a normal f32.
        (0, _) => {
            let mut exponent: i32 = 127 - 15 + 1;
            let mut mantissa = mantissa;
            while mantissa & 0x400 == 0 {
                mantissa <<= 1;
                exponent -= 1;
            }
            sign | ((exponent as u32) << 23) | ((mantissa & 0x3ff) << 13)
        }
        (0x1f, 0) => sign | 0x7f80_0000,
        (0x1f, _) => sign | 0x7fc0_0000 | (mantissa << 13),
        _ => sign | ((exponent + 127 - 15) << 23) | (mantissa << 13),
    };
    f32::from_bits(value)
}

/// Undo the two steps OpenEXR's RLE and ZIP apply before compressing:
/// the bytes delta-coded, then split into the even and the odd ones.
fn unpredict(data: &mut [u8]) -> Vec<u8> {
    for at in 1..data.len() {
        data[at] = data[at - 1].wrapping_add(data[at]).wrapping_sub(128);
    }
    let half = data.len().div_ceil(2);
    let (first, second) = data.split_at(half);
    let mut out = Vec::with_capacity(data.len());
    for (at, byte) in first.iter().enumerate() {
        out.push(*byte);
        if let Some(byte) = second.get(at) {
            out.push(*byte);
        }
    }
    out
}

/// OpenEXR's run-length coding: a signed count, then either that many
/// literal bytes (negative) or one byte repeated count plus one times.
fn unrun(data: &[u8], expected: usize) -> Result<Vec<u8>, Refusal> {
    let mut out = Vec::with_capacity(expected);
    let mut at = 0;
    while at < data.len() {
        let count = data[at] as i8;
        at += 1;
        if count < 0 {
            let count = (-i32::from(count)) as usize;
            let run = data.get(at..at + count).ok_or("an RLE run is cut short")?;
            out.extend_from_slice(run);
            at += count;
        } else {
            let byte = *data.get(at).ok_or("an RLE run is cut short")?;
            out.extend(std::iter::repeat_n(byte, count as usize + 1));
            at += 1;
        }
        if out.len() > expected {
            return Err("an RLE chunk is longer than its lines".to_string());
        }
    }
    Ok(out)
}

/// Read an OpenEXR file's red, green and blue.
pub fn read(bytes: &[u8]) -> Result<Picture, Refusal> {
    let mut file = Bytes { bytes, at: 0 };
    if file.u32()? != MAGIC {
        return Err("not an OpenEXR file".to_string());
    }
    let version = file.u32()?;
    if version & 0xff != 2 {
        return Err(format!("OpenEXR version {} is not read", version & 0xff));
    }
    let tiled = version & 0x200 != 0;
    if version & 0x800 != 0 {
        return Err("deep OpenEXR data is not read".to_string());
    }
    if version & 0x1000 != 0 {
        return Err("an OpenEXR file of several parts is not read".to_string());
    }

    let mut channels = Vec::new();
    let mut compression = None;
    let mut window = None;
    let mut tile_size = None;
    loop {
        let name = file.name()?;
        if name.is_empty() {
            break;
        }
        let kind = file.name()?;
        let size = file.i32()?;
        let value = file.take(usize::try_from(size).map_err(|_| "a negative attribute size")?)?;
        match (name.as_str(), kind.as_str()) {
            ("channels", "chlist") => channels = channel_list(value)?,
            ("compression", "compression") => {
                compression = Some(match value.first() {
                    Some(0) => Compression::None,
                    Some(1) => Compression::Rle,
                    Some(2) => Compression::Zips,
                    Some(3) => Compression::Zip,
                    Some(4) => return Err(refused("PIZ")),
                    Some(5) => return Err(refused("PXR24")),
                    Some(6) | Some(7) => return Err(refused("B44")),
                    Some(8) | Some(9) => return Err(refused("DWA")),
                    _ => return Err("an unknown OpenEXR compression".to_string()),
                })
            }
            ("tiles", "tiledesc") if value.len() == 9 => {
                let size = |at: usize| u32::from_le_bytes(value[at..at + 4].try_into().unwrap());
                tile_size = Some((size(0) as usize, size(4) as usize));
            }
            ("dataWindow", "box2i") if value.len() == 16 => {
                let corner = |at: usize| i32::from_le_bytes(value[at..at + 4].try_into().unwrap());
                window = Some((corner(0), corner(4), corner(8), corner(12)));
            }
            _ => {}
        }
    }
    let compression = compression.ok_or("the header names no compression")?;
    let (x_min, y_min, x_max, y_max) = window.ok_or("the header has no data window")?;
    if x_max < x_min || y_max < y_min {
        return Err("the data window is empty".to_string());
    }
    let width = (i64::from(x_max) - i64::from(x_min) + 1) as usize;
    let height = (i64::from(y_max) - i64::from(y_min) + 1) as usize;
    if (width as u64) * (height as u64) > MOST_PIXELS {
        return Err(format!("{width} by {height} is larger than a map is read"));
    }
    if channels.iter().any(|channel| channel.x_sampling != 1 || channel.y_sampling != 1) {
        return Err("subsampled OpenEXR channels are not read".to_string());
    }
    let pixel_bytes: usize = channels.iter().map(|channel| channel.sample.bytes()).sum();

    // Every chunk's offset, then each chunk: where its rectangle is, and
    // its bytes. A tiled file's first `across * down` offsets are its
    // full-resolution tiles; any smaller levels come after and are left.
    let layout = match (tiled, tile_size) {
        (false, _) => Layout::Lines(compression.lines_per_chunk()),
        (true, Some((tile_width, tile_height))) if tile_width > 0 && tile_height > 0 => {
            Layout::Tiles(tile_width, tile_height)
        }
        (true, _) => return Err("a tiled file with no tile size".to_string()),
    };
    let chunks = match layout {
        Layout::Lines(lines) => height.div_ceil(lines),
        Layout::Tiles(tile_width, tile_height) => {
            width.div_ceil(tile_width) * height.div_ceil(tile_height)
        }
    };
    let mut offsets = Vec::with_capacity(chunks);
    for _ in 0..chunks {
        offsets.push(file.u64()?);
    }

    // Which of the channels, in file order, is red, green or blue.
    let slot = |name: &str| match name {
        "R" => Some(0),
        "G" => Some(1),
        "B" => Some(2),
        _ => None,
    };
    let mut pixels = vec![[0.0f32; 3]; width * height];
    for offset in offsets {
        let mut chunk =
            Bytes { bytes, at: usize::try_from(offset).map_err(|_| "a chunk past the end")? };
        let (left, top, across, down) = match layout {
            Layout::Lines(lines) => {
                let first = i64::from(chunk.i32()?) - i64::from(y_min);
                if first < 0 || first as usize >= height {
                    return Err("a chunk outside the data window".to_string());
                }
                let first = first as usize;
                (0, first, width, lines.min(height - first))
            }
            Layout::Tiles(tile_width, tile_height) => {
                let (column, row) = (chunk.i32()?, chunk.i32()?);
                let (level_x, level_y) = (chunk.i32()?, chunk.i32()?);
                if (level_x, level_y) != (0, 0) || column < 0 || row < 0 {
                    continue;
                }
                let (left, top) = (column as usize * tile_width, row as usize * tile_height);
                if left >= width || top >= height {
                    return Err("a tile outside the data window".to_string());
                }
                (left, top, tile_width.min(width - left), tile_height.min(height - top))
            }
        };
        let size = usize::try_from(chunk.i32()?).map_err(|_| "a chunk of negative size")?;
        let data = chunk.take(size)?;
        let expected = pixel_bytes * across * down;
        let raw = decoded(data, expected, compression)?;
        let mut at = 0;
        for line in 0..down {
            let y = top + line;
            for channel in &channels {
                let size = channel.sample.bytes();
                let target = slot(&channel.name);
                for x in left..left + across {
                    let sample = &raw[at..at + size];
                    at += size;
                    let Some(target) = target else {
                        continue;
                    };
                    let value = match channel.sample {
                        Sample::Half => half_to_f32(u16::from_le_bytes([sample[0], sample[1]])),
                        Sample::Float => f32::from_le_bytes(sample.try_into().expect("four")),
                        Sample::Uint => u32::from_le_bytes(sample.try_into().expect("four")) as f32,
                    };
                    pixels[y * width + x][target] = value;
                }
            }
        }
    }
    if !channels.iter().any(|channel| slot(&channel.name).is_some()) {
        return Err("it has no red, green or blue channel".to_string());
    }
    Ok(Picture { width, height, pixels })
}

/// How the picture is cut into chunks: lines so many at a time, or tiles
/// so wide and so high.
#[derive(Debug, Clone, Copy)]
enum Layout {
    Lines(usize),
    Tiles(usize, usize),
}

/// A chunk's bytes as they were before compression.
fn decoded(data: &[u8], expected: usize, compression: Compression) -> Result<Vec<u8>, Refusal> {
    // Stored as it is: no compression, or compression that did not make the
    // chunk smaller, which OpenEXR then keeps raw.
    let raw = if data.len() == expected {
        data.to_vec()
    } else {
        match compression {
            Compression::None => return Err("a chunk is not the size of its lines".into()),
            Compression::Rle => unpredict(&mut unrun(data, expected)?),
            Compression::Zips | Compression::Zip => {
                let mut inflated =
                    miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(data, expected)
                        .map_err(|_| "a ZIP chunk does not inflate".to_string())?;
                unpredict(&mut inflated)
            }
        }
    };
    if raw.len() != expected {
        return Err("a chunk is not the size of its lines".to_string());
    }
    Ok(raw)
}

fn refused(compression: &str) -> Refusal {
    format!("{compression} compression is not read: save the map with ZIP")
}

/// The `chlist` attribute: each channel's name, sample type, linearity and
/// sampling, ended by an empty name.
fn channel_list(value: &[u8]) -> Result<Vec<Channel>, Refusal> {
    let mut list = Bytes { bytes: value, at: 0 };
    let mut channels = Vec::new();
    loop {
        let name = list.name()?;
        if name.is_empty() {
            return Ok(channels);
        }
        let sample = match list.i32()? {
            0 => Sample::Uint,
            1 => Sample::Half,
            2 => Sample::Float,
            other => return Err(format!("an unknown OpenEXR sample type {other}")),
        };
        list.take(4)?; // pLinear and three reserved bytes
        let x_sampling = list.i32()?;
        let y_sampling = list.i32()?;
        channels.push(Channel { name, sample, x_sampling, y_sampling });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use exr::prelude::{
        Blocks, Compression as Written, Encoding, Image, LineOrder, SpecificChannels,
        WritableImage, f16,
    };

    /// A file written by the `exr` crate, in `compression`, each pixel from
    /// `pixel`, in halves or in floats.
    fn written(
        width: usize,
        height: usize,
        compression: Written,
        half: bool,
        pixel: impl Fn(usize, usize) -> (f32, f32, f32) + Sync,
    ) -> Vec<u8> {
        written_in(width, height, compression, Blocks::ScanLines, half, pixel)
    }

    /// As [`written`], in scanlines or in tiles.
    fn written_in(
        width: usize,
        height: usize,
        compression: Written,
        blocks: Blocks,
        half: bool,
        pixel: impl Fn(usize, usize) -> (f32, f32, f32) + Sync,
    ) -> Vec<u8> {
        let encoding = Encoding { compression, blocks, line_order: LineOrder::Increasing };
        let mut bytes = std::io::Cursor::new(Vec::new());
        if half {
            let channels = SpecificChannels::rgb(|at: exr::math::Vec2<usize>| {
                let (r, g, b) = pixel(at.x(), at.y());
                (f16::from_f32(r), f16::from_f32(g), f16::from_f32(b))
            });
            Image::from_encoded_channels((width, height), encoding, channels)
                .write()
                .non_parallel()
                .to_buffered(&mut bytes)
                .expect("writing");
        } else {
            let channels =
                SpecificChannels::rgb(|at: exr::math::Vec2<usize>| pixel(at.x(), at.y()));
            Image::from_encoded_channels((width, height), encoding, channels)
                .write()
                .non_parallel()
                .to_buffered(&mut bytes)
                .expect("writing");
        }
        bytes.into_inner()
    }

    /// Something to tell every pixel and channel apart, with fractions, a
    /// negative and a long run of one value so RLE has a run to make.
    fn pattern(x: usize, y: usize) -> (f32, f32, f32) {
        if y == 3 {
            return (0.25, 0.25, 0.25);
        }
        (x as f32 * 0.5 - 3.0, y as f32 * 0.125, (x * 7 + y * 13) as f32 / 64.0)
    }

    fn rounded(value: f32) -> f32 {
        f16::from_f32(value).to_f32()
    }

    #[test]
    fn every_compression_read_is_read_exactly_in_floats_and_in_halves() {
        // In scanlines, and in tiles of 16 (which do not divide the picture)
        // and of 64 (the `exr` crate's own default).
        let (width, height) = (37, 41);
        let layouts = [
            (Blocks::ScanLines, "lines"),
            (Blocks::Tiles(exr::math::Vec2(16, 16)), "16 tiles"),
            (Blocks::Tiles(exr::math::Vec2(64, 64)), "64 tiles"),
        ];
        let compressions = [Written::Uncompressed, Written::RLE, Written::ZIP1, Written::ZIP16];
        for compression in compressions {
            for (blocks, layout) in layouts {
                for half in [false, true] {
                    let file = written_in(width, height, compression, blocks, half, pattern);
                    let picture = read(&file)
                        .unwrap_or_else(|why| panic!("{compression:?} {layout} {half}: {why}"));
                    assert_eq!((picture.width, picture.height), (width, height));
                    for y in 0..height {
                        for x in 0..width {
                            let (r, g, b) = pattern(x, y);
                            let wanted =
                                if half { [rounded(r), rounded(g), rounded(b)] } else { [r, g, b] };
                            assert_eq!(
                                picture.pixels[y * width + x],
                                wanted,
                                "{compression:?} {layout} half {half} at {x},{y}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_compression_not_read_is_refused_by_name() {
        let file = written(16, 16, Written::PIZ, false, pattern);
        let why = read(&file).expect_err("PIZ was read");
        assert!(why.contains("PIZ") && why.contains("ZIP"), "{why}");
    }

    /// Every one of the 65,536 halves, against the `half` crate's own.
    #[test]
    fn every_half_converts_as_the_half_crate_converts_it() {
        for bits in 0..=u16::MAX {
            let ours = half_to_f32(bits);
            let theirs = f16::from_bits(bits).to_f32();
            if theirs.is_nan() {
                assert!(ours.is_nan(), "{bits:#06x}");
            } else {
                assert_eq!(ours.to_bits(), theirs.to_bits(), "{bits:#06x}");
            }
        }
    }

    #[test]
    fn what_is_not_an_openexr_file_is_refused() {
        assert!(read(b"").is_err());
        assert!(read(b"\x89PNG\r\n\x1a\n").is_err());
        let file = written(8, 8, Written::ZIP16, false, pattern);
        assert!(read(&file[..file.len() / 2]).is_err(), "a file cut in half was read");
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(24))]

        /// Any size, any values, ZIP or RLE: read back as written.
        #[test]
        fn any_picture_is_read_back_as_written(
            width in 1usize..48,
            height in 1usize..48,
            seed in proptest::prelude::any::<u32>(),
            zip in proptest::prelude::any::<bool>(),
        ) {
            let value = |x: usize, y: usize, channel: usize| {
                let mixed = (x as u32 * 73_856_093) ^ (y as u32 * 19_349_663)
                    ^ (channel as u32 * 83_492_791) ^ seed;
                (mixed % 20_000) as f32 / 100.0 - 100.0
            };
            let pixel = |x: usize, y: usize| (value(x, y, 0), value(x, y, 1), value(x, y, 2));
            let compression = if zip { Written::ZIP16 } else { Written::RLE };
            let file = written(width, height, compression, false, pixel);
            let picture = read(&file).map_err(proptest::test_runner::TestCaseError::fail)?;
            for y in 0..height {
                for x in 0..width {
                    let (r, g, b) = pixel(x, y);
                    proptest::prop_assert_eq!(picture.pixels[y * width + x], [r, g, b]);
                }
            }
        }

        /// Any cut or any garbled byte: refused or read, never a panic.
        #[test]
        fn a_damaged_file_never_panics(cut in 0usize..4096, at in 0usize..4096, byte: u8) {
            let mut file = written(24, 20, Written::ZIP16, true, pattern);
            let at = at % file.len();
            file[at] = byte;
            file.truncate(cut.min(file.len()));
            let _ = read(&file);
        }
    }
}

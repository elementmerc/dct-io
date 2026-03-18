#![forbid(unsafe_code)]
//! Read and write quantized DCT coefficients in baseline JPEG files.
//!
//! This crate provides direct access to the quantized DCT coefficients stored
//! in the entropy-coded data of a baseline JPEG. It is useful for
//! steganography, watermarking, forensic analysis, and JPEG-domain signal
//! processing where you need to read or modify coefficients without fully
//! decoding the image to pixel values.
//!
//! # What this crate does
//!
//! JPEG compresses images by dividing them into 8×8 pixel blocks, applying a
//! Discrete Cosine Transform (DCT) to each block, quantizing the resulting
//! coefficients, and then entropy-coding them with Huffman coding. This crate
//! parses the entropy-coded stream, decodes the Huffman symbols, reconstructs
//! the quantized coefficient values, and lets you read or modify them before
//! re-encoding everything back into a valid JPEG byte stream.
//!
//! # What this crate does NOT do
//!
//! - It does not decode pixel values (no IDCT, no dequantisation).
//! - It does not support progressive JPEG (SOF2), lossless JPEG (SOF3), or
//!   arithmetic coding (SOF9). Passing such files returns an error.
//! - It does not support JPEG 2000.
//!
//! # Supported JPEG variants
//!
//! - Baseline DCT (SOF0) — the most common variant
//! - Extended sequential DCT (SOF1) — treated identically to SOF0
//! - Grayscale (1 component) and colour (3 components, typically YCbCr)
//! - All standard chroma subsampling ratios (4:4:4, 4:2:2, 4:2:0, etc.)
//! - EXIF and JFIF headers
//! - Restart markers (DRI / RST0–RST7)
//!
//! # Example
//!
//! ```no_run
//! use dct_io::{read_coefficients, write_coefficients};
//!
//! let jpeg = std::fs::read("photo.jpg").unwrap();
//!
//! let mut coeffs = read_coefficients(&jpeg).unwrap();
//!
//! // Flip the LSB of every eligible AC coefficient in the first component.
//! for block in &mut coeffs.components[0].blocks {
//!     for coeff in block[1..].iter_mut() {
//!         if *coeff != 0 {
//!             *coeff ^= 1;
//!         }
//!     }
//! }
//!
//! let modified = write_coefficients(&jpeg, &coeffs).unwrap();
//! std::fs::write("photo_modified.jpg", modified).unwrap();
//! ```

use thiserror::Error;

// ── Public error type ─────────────────────────────────────────────────────────

/// Errors returned by this crate.
#[derive(Debug, Error)]
pub enum DctError {
    /// The input does not start with a JPEG SOI marker (`0xFF 0xD8`).
    #[error("not a JPEG file")]
    NotJpeg,

    /// The input was truncated mid-marker or mid-entropy-stream.
    #[error("truncated JPEG data")]
    Truncated,

    /// The entropy-coded data contains an invalid Huffman symbol or an
    /// unexpected structure.
    #[error("corrupt or malformed JPEG entropy stream")]
    CorruptEntropy,

    /// The JPEG uses a feature this crate does not support (e.g. progressive
    /// scan, lossless, or arithmetic coding).
    #[error("unsupported JPEG variant: {0}")]
    Unsupported(String),

    /// A required marker or table is missing from the JPEG (e.g. no SOF, no
    /// SOS, or a scan references a Huffman table that was not defined).
    #[error("missing required JPEG structure: {0}")]
    Missing(String),

    /// The `JpegCoefficients` passed to [`write_coefficients`] is not
    /// compatible with the JPEG (wrong number of components, wrong block
    /// count, wrong component index).
    #[error("coefficient data is incompatible with this JPEG: {0}")]
    Incompatible(String),
}

// ── Public types ──────────────────────────────────────────────────────────────

/// Metadata for a single image component, as read from the SOF marker.
#[derive(Debug, Clone)]
pub struct ComponentInfo {
    /// Component identifier (1=Y, 2=Cb, 3=Cr in YCbCr; 1=Y in grayscale).
    pub id: u8,
    /// Horizontal sampling factor.
    pub h_samp: u8,
    /// Vertical sampling factor.
    pub v_samp: u8,
    /// Number of 8×8 DCT blocks this component contributes to the image.
    pub block_count: usize,
}

/// Image metadata extracted from a JPEG without decoding the entropy stream.
///
/// Obtained from [`inspect`]. Cheaper than [`read_coefficients`] when you
/// only need dimensions, component count, or block counts.
#[derive(Debug, Clone)]
pub struct JpegInfo {
    /// Image width in pixels.
    pub width: u16,
    /// Image height in pixels.
    pub height: u16,
    /// Per-component metadata, in SOF order (typically Y, Cb, Cr).
    pub components: Vec<ComponentInfo>,
}

/// Quantized DCT coefficients for a single component (Y, Cb, or Cr).
///
/// Each element of `blocks` is one 8×8 DCT block, stored in the JPEG zigzag
/// scan order:
/// - Index 0: DC coefficient (top-left of the frequency matrix).
/// - Indices 1–63: AC coefficients in zigzag order.
///
/// The values are the quantized coefficients exactly as they appear in the
/// JPEG bitstream. They have **not** been dequantized; multiply by the
/// quantization table to recover the pre-quantized DCT values.
#[derive(Debug, Clone)]
pub struct ComponentCoefficients {
    /// Component identifier as written in the JPEG SOF marker.
    pub id: u8,
    /// All 8×8 blocks for this component, in raster scan order (left-to-right,
    /// top-to-bottom). Each block contains exactly 64 `i16` values.
    pub blocks: Vec<[i16; 64]>,
}

/// Quantized DCT coefficients for all components in a JPEG image.
///
/// Returned by [`read_coefficients`] and accepted by [`write_coefficients`].
#[derive(Debug, Clone)]
pub struct JpegCoefficients {
    /// One entry per component, in the order they appear in the JPEG SOF
    /// marker (typically Y, Cb, Cr for colour images).
    pub components: Vec<ComponentCoefficients>,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Decode the quantized DCT coefficients from a baseline JPEG.
///
/// Returns [`JpegCoefficients`] containing all blocks for all components.
/// Does not dequantize or apply IDCT; values are the raw quantized integers.
///
/// # Errors
///
/// Returns [`DctError`] if the input is not a supported baseline JPEG, if
/// required markers are missing, or if the entropy stream is corrupt.
#[must_use = "returns the decoded coefficients or an error; ignoring it discards the result"]
pub fn read_coefficients(jpeg: &[u8]) -> Result<JpegCoefficients, DctError> {
    let mut parser = JpegParser::new(jpeg)?;
    parser.parse()?;
    parser.decode_coefficients()
}

/// Re-encode a JPEG with modified DCT coefficients.
///
/// Takes the original JPEG bytes and a [`JpegCoefficients`] (typically
/// obtained from [`read_coefficients`] and then modified), and produces a new
/// JPEG byte stream with the updated coefficients re-encoded using the same
/// Huffman tables as the original.
///
/// The output is a valid JPEG. All non-entropy-coded segments (EXIF, ICC
/// profile, quantization tables, etc.) are preserved verbatim.
///
/// # Safety note
///
/// The output is only as valid as the input JPEG's Huffman tables permit.
/// If you set a coefficient to a value whose (run, category) symbol does not
/// exist in the original Huffman table, encoding will return
/// [`DctError::CorruptEntropy`]. Stick to modifying the LSB of coefficients
/// with `|v| >= 2` (JSteg-style) to stay safely within the table.
///
/// # Errors
///
/// Returns [`DctError::Incompatible`] if `coeffs` has a different number of
/// components, a different block count, or mismatched component IDs compared
/// to the original JPEG.
/// Returns [`DctError`] for any parse or encoding failure.
#[must_use = "returns the re-encoded JPEG bytes or an error; ignoring it discards the result"]
pub fn write_coefficients(jpeg: &[u8], coeffs: &JpegCoefficients) -> Result<Vec<u8>, DctError> {
    let mut parser = JpegParser::new(jpeg)?;
    parser.parse()?;
    parser.encode_coefficients(jpeg, coeffs)
}

/// Return the number of 8×8 DCT blocks per component in a JPEG.
///
/// The returned `Vec` has one entry per component (in SOF order). Useful
/// for determining how many blocks are available before calling
/// [`read_coefficients`].
///
/// # Errors
///
/// Returns [`DctError`] if the input is not a supported baseline JPEG.
#[must_use = "returns block counts or an error; ignoring it discards the result"]
pub fn block_count(jpeg: &[u8]) -> Result<Vec<usize>, DctError> {
    let mut parser = JpegParser::new(jpeg)?;
    parser.parse()?;
    parser.block_counts()
}

/// Inspect a JPEG and return image metadata without decoding the entropy stream.
///
/// Much cheaper than [`read_coefficients`] when you only need the image
/// dimensions, component layout, or block counts.
///
/// # Errors
///
/// Returns [`DctError`] if the input is not a supported baseline JPEG.
#[must_use = "returns image metadata or an error; ignoring it discards the result"]
pub fn inspect(jpeg: &[u8]) -> Result<JpegInfo, DctError> {
    let mut parser = JpegParser::new(jpeg)?;
    parser.parse()?;
    let counts = parser.block_counts()?;
    Ok(JpegInfo {
        width: parser.image_width,
        height: parser.image_height,
        components: parser
            .frame_components
            .iter()
            .enumerate()
            .map(|(i, fc)| ComponentInfo {
                id: fc.id,
                h_samp: fc.h_samp,
                v_samp: fc.v_samp,
                block_count: counts[i],
            })
            .collect(),
    })
}

/// Count the number of AC coefficients with `|v| >= 2` across all components.
///
/// These are the coefficients that can be modified without altering zero-run
/// lengths or EOB positions — the eligible positions for JSteg-style LSB
/// embedding. Decodes all coefficients internally; use
/// [`JpegCoefficients::eligible_ac_count`] to avoid decoding twice.
///
/// # Errors
///
/// Returns [`DctError`] if the input is not a supported baseline JPEG.
#[must_use = "returns the eligible AC coefficient count or an error; ignoring it discards the result"]
pub fn eligible_ac_count(jpeg: &[u8]) -> Result<usize, DctError> {
    Ok(read_coefficients(jpeg)?.eligible_ac_count())
}

impl JpegCoefficients {
    /// Count the number of AC coefficients with `|v| >= 2` across all
    /// components.
    ///
    /// Modifying only these coefficients preserves the zero-run structure of
    /// the entropy stream, keeping the output a valid JPEG that is
    /// perceptually indistinguishable from the original.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use dct_io::read_coefficients;
    ///
    /// let jpeg = std::fs::read("photo.jpg").unwrap();
    /// let coeffs = read_coefficients(&jpeg).unwrap();
    /// println!("Eligible AC positions: {}", coeffs.eligible_ac_count());
    /// ```
    #[must_use]
    pub fn eligible_ac_count(&self) -> usize {
        self.components
            .iter()
            .flat_map(|c| c.blocks.iter())
            .flat_map(|b| b[1..].iter())
            .filter(|&&v| v.abs() >= 2)
            .count()
    }
}

// ── Internal constants ────────────────────────────────────────────────────────

/// Zigzag scan order: maps coefficient index (0..64) to (row, col) in an 8×8
/// block, expressed as a flat index `row*8 + col`.
#[rustfmt::skip]
const ZIGZAG: [u8; 64] = [
     0,  1,  8, 16,  9,  2,  3, 10,
    17, 24, 32, 25, 18, 11,  4,  5,
    12, 19, 26, 33, 40, 48, 41, 34,
    27, 20, 13,  6,  7, 14, 21, 28,
    35, 42, 49, 56, 57, 50, 43, 36,
    29, 22, 15, 23, 30, 37, 44, 51,
    58, 59, 52, 45, 38, 31, 39, 46,
    53, 60, 61, 54, 47, 55, 62, 63,
];

/// Maximum number of MCUs we are willing to decode (safety cap).
const MAX_MCU_COUNT: usize = 1_048_576; // 1M MCUs ~ 67 megapixels at 4:2:0

// ── Value-category helper ─────────────────────────────────────────────────────

/// JPEG value category: the number of bits needed to represent `abs(v)`.
/// Category 0 is special (used for the zero DC difference and the EOB symbol).
/// Capped at 15 to guard against malformed input.
#[inline]
fn category(value: i16) -> u8 {
    if value == 0 {
        return 0;
    }
    let abs = value.unsigned_abs();
    let cat = (16u32 - abs.leading_zeros()) as u8;
    cat.min(15)
}

/// Encode `value` into its (category, magnitude bits) JPEG representation.
/// Returns `(cat, bits, bit_count)`.
#[inline]
fn encode_value(value: i16) -> (u8, u16, u8) {
    let cat = category(value);
    if cat == 0 {
        return (0, 0, 0);
    }
    let bits = if value > 0 {
        value as u16
    } else {
        // Negative: encode as (2^cat - 1 + value)
        let v = (1i16 << cat) - 1 + value;
        v as u16
    };
    (cat, bits, cat)
}

// ── Huffman table ─────────────────────────────────────────────────────────────

/// A single Huffman table (DC or AC, for one component class).
///
/// Decoding uses a flat 65 536-entry lookup table indexed by the top 16 bits
/// of the bit-stream. Each entry packs `(symbol << 8) | code_len` as a `u16`,
/// with 0 meaning "no code with this prefix". This gives O(1) decode with no
/// branch on the hot path.
///
/// Encoding uses a flat 256-entry array keyed by symbol (u8). Each entry is
/// `(code, code_length)`; a length of 0 means the symbol is not in this table.
#[derive(Clone)]
struct HuffTable {
    /// 16-bit LUT: index = top 16 stream bits → `(symbol << 8) | len`, 0 = invalid.
    lut: Vec<u16>,
    /// Encode table: `encode[symbol] = (code, code_length)`, len 0 = absent.
    encode: [(u16, u8); 256],
}

impl std::fmt::Debug for HuffTable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let entries = self.encode.iter().filter(|e| e.1 > 0).count();
        f.debug_struct("HuffTable")
            .field("encode_entries", &entries)
            .finish()
    }
}

impl HuffTable {
    /// Build a Huffman table from the DHT segment payload.
    ///
    /// `counts` is the 16-byte array of code counts per length (1..=16).
    /// `symbols` is the flat list of symbols in canonical order.
    fn from_jpeg(counts: &[u8; 16], symbols: &[u8]) -> Result<Self, DctError> {
        let mut encode = [(0u16, 0u8); 256];
        let mut lut = vec![0u16; 65536];
        let mut code: u16 = 0;
        let mut sym_idx = 0usize;

        for len in 1u8..=16u8 {
            let count = counts[(len - 1) as usize] as usize;
            for _ in 0..count {
                if sym_idx >= symbols.len() {
                    return Err(DctError::CorruptEntropy);
                }
                // Guard against a malformed DHT where the canonical code would
                // overflow 16 bits or index outside our LUT. Use u32 for the
                // shift so `len == 16` does not itself overflow.
                if (code as u32) >= (1u32 << len) {
                    return Err(DctError::CorruptEntropy);
                }
                let sym = symbols[sym_idx];
                sym_idx += 1;
                encode[sym as usize] = (code, len);

                // Fill all 16-bit keys whose top `len` bits equal `code`.
                // Each such key represents a stream where the Huffman prefix
                // is followed by arbitrary suffix bits.
                let spread = 1usize << (16 - len);
                let base = (code as usize) << (16 - len);
                let entry = ((sym as u16) << 8) | (len as u16);
                lut[base..base + spread].fill(entry);

                code += 1;
            }
            code <<= 1;
        }

        Ok(HuffTable { lut, encode })
    }
}

// ── Bit reader ────────────────────────────────────────────────────────────────

struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    buf: u64,
    bits: u8,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        BitReader {
            data,
            pos: 0,
            buf: 0,
            bits: 0,
        }
    }

    /// Fill `buf` from the entropy stream, skipping byte stuffing (0xFF 0x00)
    /// and stopping at any marker (0xFF 0xD0–0xD9 or any non-0x00 after 0xFF).
    fn refill(&mut self) {
        while self.bits <= 56 {
            if self.pos >= self.data.len() {
                break;
            }
            let byte = self.data[self.pos];
            if byte == 0xFF {
                if self.pos + 1 >= self.data.len() {
                    break;
                }
                let next = self.data[self.pos + 1];
                if next == 0x00 {
                    // Byte stuffing — consume both, emit 0xFF.
                    self.pos += 2;
                    self.buf = (self.buf << 8) | 0xFF;
                    self.bits += 8;
                } else {
                    // Marker — stop refilling.
                    break;
                }
            } else {
                self.pos += 1;
                self.buf = (self.buf << 8) | (byte as u64);
                self.bits += 8;
            }
        }
    }

    /// Peek at the top `n` bits without consuming them.
    fn peek(&mut self, n: u8) -> Result<u16, DctError> {
        if self.bits < n {
            self.refill();
        }
        if self.bits < n {
            return Err(DctError::Truncated);
        }
        Ok(((self.buf >> (self.bits - n)) & ((1u64 << n) - 1)) as u16)
    }

    /// Consume `n` bits.
    fn consume(&mut self, n: u8) {
        debug_assert!(self.bits >= n);
        self.bits -= n;
        self.buf &= (1u64 << self.bits) - 1;
    }

    /// Read `n` bits and return them as a `u16`.
    fn read_bits(&mut self, n: u8) -> Result<u16, DctError> {
        if n == 0 {
            return Ok(0);
        }
        let v = self.peek(n)?;
        self.consume(n);
        Ok(v)
    }

    /// Decode the next Huffman symbol using the 16-bit LUT.
    ///
    /// Forms a 16-bit key from the top bits of the buffer (right-padded with
    /// zeros if fewer than 16 bits are available). The LUT maps this key
    /// directly to `(symbol, code_length)` in a single indexed read.
    fn decode_huffman(&mut self, table: &HuffTable) -> Result<u8, DctError> {
        if self.bits < 16 {
            self.refill();
        }
        // Build the 16-bit key: top `min(bits, 16)` stream bits left-aligned.
        let key = if self.bits >= 16 {
            ((self.buf >> (self.bits - 16)) & 0xFFFF) as u16
        } else {
            // Fewer than 16 bits available — pad the right with zeros.
            // The LUT entry for any short code covers all possible suffixes,
            // so zero-padding is safe as long as len <= self.bits.
            ((self.buf << (16 - self.bits)) & 0xFFFF) as u16
        };

        let entry = table.lut[key as usize];
        let len = (entry & 0xFF) as u8;
        let sym = (entry >> 8) as u8;

        if len == 0 {
            return Err(DctError::CorruptEntropy);
        }
        if self.bits < len {
            return Err(DctError::Truncated);
        }
        self.consume(len);
        Ok(sym)
    }

    /// Skip any restart marker at the current position and reset DC predictor.
    /// Returns `true` if a restart marker was consumed.
    fn sync_restart(&mut self) -> bool {
        // Discard any remaining bits in the current byte.
        self.bits = 0;
        self.buf = 0;
        // Check for a single 0xFF followed by RST0–RST7 (0xD0–0xD7).
        if self.pos + 1 < self.data.len()
            && self.data[self.pos] == 0xFF
            && (0xD0..=0xD7).contains(&self.data[self.pos + 1])
        {
            self.pos += 2;
            return true;
        }
        false
    }
}

// ── Bit writer ────────────────────────────────────────────────────────────────

struct BitWriter {
    out: Vec<u8>,
    buf: u64,
    bits: u8,
}

impl BitWriter {
    fn with_capacity(cap: usize) -> Self {
        BitWriter {
            out: Vec::with_capacity(cap),
            buf: 0,
            bits: 0,
        }
    }

    /// Write `n` bits of `value` (MSB first).
    fn write_bits(&mut self, value: u16, n: u8) {
        if n == 0 {
            return;
        }
        self.buf = (self.buf << n) | (value as u64);
        self.bits += n;
        while self.bits >= 8 {
            self.bits -= 8;
            let byte = ((self.buf >> self.bits) & 0xFF) as u8;
            self.out.push(byte);
            if byte == 0xFF {
                self.out.push(0x00); // Byte stuffing.
            }
            self.buf &= (1u64 << self.bits) - 1;
        }
    }

    /// Flush any remaining bits (padded with 1-bits per the JPEG spec).
    fn flush(&mut self) {
        if self.bits > 0 {
            let pad = 8 - self.bits;
            let byte = (((self.buf << pad) | ((1u64 << pad) - 1)) & 0xFF) as u8;
            self.out.push(byte);
            if byte == 0xFF {
                self.out.push(0x00);
            }
            self.bits = 0;
            self.buf = 0;
        }
    }

    /// Emit a restart marker (0xFF 0xDn) directly into the output without
    /// byte-stuffing (markers are not entropy data).
    fn write_restart_marker(&mut self, n: u8) {
        self.flush();
        self.out.push(0xFF);
        self.out.push(0xD0 | (n & 0x07));
    }
}

// ── Internal JPEG parser ──────────────────────────────────────────────────────

/// Metadata for one component as read from the SOF marker.
#[derive(Debug, Clone)]
struct FrameComponent {
    id: u8,
    h_samp: u8,
    v_samp: u8,
    #[allow(dead_code)]
    qt_id: u8,
}

/// Per-component data from the SOS marker.
#[derive(Debug, Clone)]
struct ScanComponent {
    comp_idx: usize, // index into frame_components
    dc_table: usize,
    ac_table: usize,
}

/// Parsed state accumulated while scanning JPEG markers.
struct JpegParser<'a> {
    data: &'a [u8],
    pos: usize,

    /// Byte offset of the first entropy-coded data byte.
    entropy_start: usize,
    /// Byte length of the entropy-coded segment (up to next non-RST marker).
    entropy_len: usize,

    frame_components: Vec<FrameComponent>,
    scan_components: Vec<ScanComponent>,
    dc_tables: [Option<HuffTable>; 4],
    ac_tables: [Option<HuffTable>; 4],
    restart_interval: u16,
    image_width: u16,
    image_height: u16,
}

impl<'a> JpegParser<'a> {
    fn new(data: &'a [u8]) -> Result<Self, DctError> {
        if data.len() < 2 || data[0] != 0xFF || data[1] != 0xD8 {
            return Err(DctError::NotJpeg);
        }
        Ok(JpegParser {
            data,
            pos: 2,
            entropy_start: 0,
            entropy_len: 0,
            frame_components: Vec::new(),
            scan_components: Vec::new(),
            dc_tables: [None, None, None, None],
            ac_tables: [None, None, None, None],
            restart_interval: 0,
            image_width: 0,
            image_height: 0,
        })
    }

    /// Read a 2-byte big-endian u16 from `data[pos..]`, advancing `pos`.
    fn read_u16(&mut self) -> Result<u16, DctError> {
        if self.pos + 1 >= self.data.len() {
            return Err(DctError::Truncated);
        }
        let v = u16::from_be_bytes([self.data[self.pos], self.data[self.pos + 1]]);
        self.pos += 2;
        Ok(v)
    }

    /// Parse all JPEG markers up to and including SOS. Sets `entropy_start`
    /// and `entropy_len`.
    fn parse(&mut self) -> Result<(), DctError> {
        loop {
            // Find next marker.
            if self.pos >= self.data.len() {
                return Err(DctError::Missing("SOS marker".into()));
            }
            if self.data[self.pos] != 0xFF {
                return Err(DctError::CorruptEntropy);
            }
            // Skip 0xFF padding.
            while self.pos < self.data.len() && self.data[self.pos] == 0xFF {
                self.pos += 1;
            }
            if self.pos >= self.data.len() {
                return Err(DctError::Truncated);
            }
            let marker = self.data[self.pos];
            self.pos += 1;

            match marker {
                0xD8 => {} // SOI — already consumed.
                0xD9 => return Err(DctError::Missing("SOS before EOI".into())),

                // SOF0 (baseline) and SOF1 (extended sequential) — supported.
                0xC0 | 0xC1 => self.parse_sof()?,

                // SOF markers we reject with a clear message.
                0xC2 => return Err(DctError::Unsupported("progressive JPEG (SOF2)".into())),
                0xC3 => return Err(DctError::Unsupported("lossless JPEG (SOF3)".into())),
                0xC9 => return Err(DctError::Unsupported("arithmetic coding (SOF9)".into())),
                0xCA => {
                    return Err(DctError::Unsupported(
                        "progressive arithmetic (SOF10)".into(),
                    ))
                }
                0xCB => return Err(DctError::Unsupported("lossless arithmetic (SOF11)".into())),

                0xC4 => self.parse_dht()?,
                0xDD => self.parse_dri()?,

                0xDA => {
                    // SOS — parse header, then record entropy start.
                    self.parse_sos_header()?;
                    self.entropy_start = self.pos;
                    self.entropy_len = self.find_entropy_end();
                    return Ok(());
                }

                // Any other marker with a length field — skip.
                _ => {
                    let len = self.read_u16()? as usize;
                    if len < 2 {
                        return Err(DctError::CorruptEntropy);
                    }
                    let skip = len - 2;
                    if self.pos + skip > self.data.len() {
                        return Err(DctError::Truncated);
                    }
                    self.pos += skip;
                }
            }
        }
    }

    fn parse_sof(&mut self) -> Result<(), DctError> {
        let len = self.read_u16()? as usize;
        if len < 8 {
            return Err(DctError::CorruptEntropy);
        }
        let end = self.pos + len - 2;
        if end > self.data.len() {
            return Err(DctError::Truncated);
        }
        let _precision = self.data[self.pos];
        self.pos += 1;
        self.image_height = u16::from_be_bytes([self.data[self.pos], self.data[self.pos + 1]]);
        self.pos += 2;
        self.image_width = u16::from_be_bytes([self.data[self.pos], self.data[self.pos + 1]]);
        self.pos += 2;

        if self.image_width == 0 || self.image_height == 0 {
            return Err(DctError::Unsupported("zero image dimension".into()));
        }

        let ncomp = self.data[self.pos] as usize;
        self.pos += 1;

        if ncomp == 0 || ncomp > 4 {
            return Err(DctError::Unsupported(format!("{ncomp} components")));
        }
        if self.pos + ncomp * 3 > end {
            return Err(DctError::Truncated);
        }

        self.frame_components.clear();
        for _ in 0..ncomp {
            let id = self.data[self.pos];
            let samp = self.data[self.pos + 1];
            let qt_id = self.data[self.pos + 2];
            self.pos += 3;
            let h_samp = samp >> 4;
            let v_samp = samp & 0x0F;
            if h_samp == 0 || v_samp == 0 {
                return Err(DctError::CorruptEntropy);
            }
            self.frame_components.push(FrameComponent {
                id,
                h_samp,
                v_samp,
                qt_id,
            });
        }
        self.pos = end;
        Ok(())
    }

    fn parse_dht(&mut self) -> Result<(), DctError> {
        let len = self.read_u16()? as usize;
        if len < 2 {
            return Err(DctError::CorruptEntropy);
        }
        let end = self.pos + len - 2;
        if end > self.data.len() {
            return Err(DctError::Truncated);
        }

        while self.pos < end {
            if self.pos >= self.data.len() {
                return Err(DctError::Truncated);
            }
            let tc_th = self.data[self.pos];
            self.pos += 1;
            let tc = (tc_th >> 4) & 0x0F; // 0=DC, 1=AC
            let th = (tc_th & 0x0F) as usize; // table index 0–3

            if tc > 1 {
                return Err(DctError::CorruptEntropy);
            }
            if th > 3 {
                return Err(DctError::CorruptEntropy);
            }

            if self.pos + 16 > end {
                return Err(DctError::Truncated);
            }
            let mut counts = [0u8; 16];
            counts.copy_from_slice(&self.data[self.pos..self.pos + 16]);
            self.pos += 16;

            let total: usize = counts.iter().map(|&c| c as usize).sum();
            // JPEG Huffman symbols are u8, so at most 256 unique symbols per table.
            if total > 256 {
                return Err(DctError::CorruptEntropy);
            }
            if self.pos + total > end {
                return Err(DctError::Truncated);
            }
            let symbols = &self.data[self.pos..self.pos + total];
            self.pos += total;

            let table = HuffTable::from_jpeg(&counts, symbols)?;
            if tc == 0 {
                self.dc_tables[th] = Some(table);
            } else {
                self.ac_tables[th] = Some(table);
            }
        }

        self.pos = end;
        Ok(())
    }

    fn parse_dri(&mut self) -> Result<(), DctError> {
        let len = self.read_u16()?;
        if len != 4 {
            return Err(DctError::CorruptEntropy);
        }
        self.restart_interval = self.read_u16()?;
        Ok(())
    }

    fn parse_sos_header(&mut self) -> Result<(), DctError> {
        let len = self.read_u16()? as usize;
        if len < 3 {
            return Err(DctError::CorruptEntropy);
        }
        let end = self.pos + len - 2;
        if end > self.data.len() {
            return Err(DctError::Truncated);
        }

        let ns = self.data[self.pos] as usize;
        self.pos += 1;

        if ns == 0 || ns > self.frame_components.len() {
            return Err(DctError::CorruptEntropy);
        }
        if self.pos + ns * 2 > end {
            return Err(DctError::Truncated);
        }

        self.scan_components.clear();
        for _ in 0..ns {
            let comp_id = self.data[self.pos];
            let td_ta = self.data[self.pos + 1];
            self.pos += 2;

            let dc_table = (td_ta >> 4) as usize;
            let ac_table = (td_ta & 0x0F) as usize;

            if dc_table > 3 || ac_table > 3 {
                return Err(DctError::CorruptEntropy);
            }

            let comp_idx = self
                .frame_components
                .iter()
                .position(|fc| fc.id == comp_id)
                .ok_or_else(|| DctError::Missing(format!("component id {comp_id} in frame")))?;

            self.scan_components.push(ScanComponent {
                comp_idx,
                dc_table,
                ac_table,
            });
        }

        // Skip Ss, Se, Ah/Al (3 bytes).
        self.pos = end;
        Ok(())
    }

    /// Find the length of the entropy-coded segment by scanning for a marker
    /// that is not RST0–RST7 (0xD0–0xD7).
    fn find_entropy_end(&self) -> usize {
        let mut i = self.entropy_start;
        while i < self.data.len() {
            if self.data[i] == 0xFF && i + 1 < self.data.len() {
                let next = self.data[i + 1];
                if next == 0x00 {
                    // Byte stuffing.
                    i += 2;
                    continue;
                }
                if (0xD0..=0xD7).contains(&next) {
                    // RST marker inside entropy data — skip it.
                    i += 2;
                    continue;
                }
                // Real marker — entropy stream ends here.
                return i - self.entropy_start;
            }
            i += 1;
        }
        self.data.len() - self.entropy_start
    }

    // ── MCU geometry helpers ──────────────────────────────────────────────────

    fn max_h_samp(&self) -> u8 {
        self.frame_components
            .iter()
            .map(|c| c.h_samp)
            .max()
            .unwrap_or(1)
    }

    fn max_v_samp(&self) -> u8 {
        self.frame_components
            .iter()
            .map(|c| c.v_samp)
            .max()
            .unwrap_or(1)
    }

    fn mcu_cols(&self) -> usize {
        let max_h = self.max_h_samp() as usize;
        (self.image_width as usize + max_h * 8 - 1) / (max_h * 8)
    }

    fn mcu_rows(&self) -> usize {
        let max_v = self.max_v_samp() as usize;
        (self.image_height as usize + max_v * 8 - 1) / (max_v * 8)
    }

    fn mcu_count(&self) -> Result<usize, DctError> {
        self.mcu_cols()
            .checked_mul(self.mcu_rows())
            .ok_or_else(|| DctError::Unsupported("image dimensions overflow usize".into()))
    }

    /// Number of 8×8 data units per MCU for each scan component.
    fn du_per_mcu(&self) -> Vec<usize> {
        self.scan_components
            .iter()
            .map(|sc| {
                let fc = &self.frame_components[sc.comp_idx];
                (fc.h_samp as usize) * (fc.v_samp as usize)
            })
            .collect()
    }

    /// Total block count per frame component (after all scan components resolved).
    fn block_counts(&self) -> Result<Vec<usize>, DctError> {
        let n_mcu = self.mcu_count()?;
        let du = self.du_per_mcu();
        let mut counts = vec![0usize; self.frame_components.len()];
        for (sc_idx, sc) in self.scan_components.iter().enumerate() {
            counts[sc.comp_idx] = n_mcu * du[sc_idx];
        }
        Ok(counts)
    }

    // ── Decode ────────────────────────────────────────────────────────────────

    fn decode_coefficients(&self) -> Result<JpegCoefficients, DctError> {
        let entropy = &self.data[self.entropy_start..self.entropy_start + self.entropy_len];
        let n_mcu = self.mcu_count()?;

        if n_mcu > MAX_MCU_COUNT {
            return Err(DctError::Unsupported(format!(
                "image too large ({n_mcu} MCUs; max {MAX_MCU_COUNT})"
            )));
        }

        let du = self.du_per_mcu();

        // Pre-allocate output vectors.
        let counts = self.block_counts()?;
        let mut comp_blocks: Vec<Vec<[i16; 64]>> =
            counts.iter().map(|&c| vec![[0i16; 64]; c]).collect();
        let mut comp_block_idx: Vec<usize> = vec![0; self.frame_components.len()];

        let mut dc_pred: Vec<i16> = vec![0; self.scan_components.len()];
        let mut reader = BitReader::new(entropy);

        let restart_interval = self.restart_interval as usize;

        for mcu_idx in 0..n_mcu {
            // Handle restart markers.
            if restart_interval > 0 && mcu_idx > 0 && mcu_idx % restart_interval == 0 {
                reader.sync_restart();
                for p in dc_pred.iter_mut() {
                    *p = 0;
                }
            }

            for (sc_idx, sc) in self.scan_components.iter().enumerate() {
                let dc_table = self.dc_tables[sc.dc_table]
                    .as_ref()
                    .ok_or_else(|| DctError::Missing(format!("DC table {}", sc.dc_table)))?;
                let ac_table = self.ac_tables[sc.ac_table]
                    .as_ref()
                    .ok_or_else(|| DctError::Missing(format!("AC table {}", sc.ac_table)))?;

                for _du_i in 0..du[sc_idx] {
                    let mut block = [0i16; 64];

                    // DC coefficient.
                    let dc_cat = reader.decode_huffman(dc_table)?;
                    let dc_cat = dc_cat.min(15);
                    let dc_bits = reader.read_bits(dc_cat)?;
                    let dc_diff = decode_magnitude(dc_cat, dc_bits);
                    dc_pred[sc_idx] = dc_pred[sc_idx].saturating_add(dc_diff);
                    block[ZIGZAG[0] as usize] = dc_pred[sc_idx];

                    // AC coefficients.
                    let mut k = 1usize;
                    while k < 64 {
                        let rs = reader.decode_huffman(ac_table)?;
                        if rs == 0x00 {
                            // EOB — rest of block is zero.
                            break;
                        }
                        if rs == 0xF0 {
                            // ZRL — 16 zeros.
                            k += 16;
                            continue;
                        }
                        let run = (rs >> 4) as usize;
                        let cat = (rs & 0x0F).min(15);
                        k += run;
                        if k >= 64 {
                            break;
                        }
                        let bits = reader.read_bits(cat)?;
                        let val = decode_magnitude(cat, bits);
                        block[ZIGZAG[k] as usize] = val;
                        k += 1;
                    }

                    let block_idx = comp_block_idx[sc.comp_idx];
                    if block_idx >= comp_blocks[sc.comp_idx].len() {
                        return Err(DctError::CorruptEntropy);
                    }
                    comp_blocks[sc.comp_idx][block_idx] = block;
                    comp_block_idx[sc.comp_idx] += 1;
                }
            }
        }

        let components = self
            .frame_components
            .iter()
            .zip(comp_blocks)
            .map(|(fc, blocks)| ComponentCoefficients { id: fc.id, blocks })
            .collect();

        Ok(JpegCoefficients { components })
    }

    // ── Encode ────────────────────────────────────────────────────────────────

    fn encode_coefficients(
        &self,
        original: &[u8],
        coeffs: &JpegCoefficients,
    ) -> Result<Vec<u8>, DctError> {
        // Validate compatibility.
        if coeffs.components.len() != self.frame_components.len() {
            return Err(DctError::Incompatible(format!(
                "expected {} components, got {}",
                self.frame_components.len(),
                coeffs.components.len()
            )));
        }
        let counts = self.block_counts()?;
        for (i, (cc, &expected)) in coeffs.components.iter().zip(counts.iter()).enumerate() {
            if cc.id != self.frame_components[i].id {
                return Err(DctError::Incompatible(format!(
                    "component {i}: expected id {}, got {}",
                    self.frame_components[i].id, cc.id
                )));
            }
            if cc.blocks.len() != expected {
                return Err(DctError::Incompatible(format!(
                    "component {i}: expected {expected} blocks, got {}",
                    cc.blocks.len()
                )));
            }
        }

        let n_mcu = self.mcu_count()?;
        let du = self.du_per_mcu();

        let mut writer = BitWriter::with_capacity(self.entropy_len);
        let mut dc_pred: Vec<i16> = vec![0; self.scan_components.len()];
        let mut comp_block_idx: Vec<usize> = vec![0; self.frame_components.len()];
        let restart_interval = self.restart_interval as usize;
        let mut rst_count: u8 = 0;

        for mcu_idx in 0..n_mcu {
            if restart_interval > 0 && mcu_idx > 0 && mcu_idx % restart_interval == 0 {
                writer.write_restart_marker(rst_count);
                rst_count = rst_count.wrapping_add(1) & 0x07;
                for p in dc_pred.iter_mut() {
                    *p = 0;
                }
            }

            for (sc_idx, sc) in self.scan_components.iter().enumerate() {
                let dc_table = self.dc_tables[sc.dc_table]
                    .as_ref()
                    .ok_or_else(|| DctError::Missing(format!("DC table {}", sc.dc_table)))?;
                let ac_table = self.ac_tables[sc.ac_table]
                    .as_ref()
                    .ok_or_else(|| DctError::Missing(format!("AC table {}", sc.ac_table)))?;

                for _du_i in 0..du[sc_idx] {
                    let block = &coeffs.components[sc.comp_idx].blocks[comp_block_idx[sc.comp_idx]];
                    comp_block_idx[sc.comp_idx] += 1;

                    // DC coefficient.
                    let dc_val = block[ZIGZAG[0] as usize];
                    let dc_diff = dc_val.saturating_sub(dc_pred[sc_idx]);
                    dc_pred[sc_idx] = dc_val;
                    let (dc_cat, dc_bits, dc_n) = encode_value(dc_diff);
                    let (dc_code, dc_code_len) = {
                        let e = dc_table.encode[dc_cat as usize];
                        if e.1 == 0 {
                            return Err(DctError::CorruptEntropy);
                        }
                        e
                    };
                    writer.write_bits(dc_code, dc_code_len);
                    writer.write_bits(dc_bits, dc_n);

                    // AC coefficients.
                    // Find last non-zero AC position in zigzag order.
                    let last_nonzero_zz = (1..64).rev().find(|&i| block[ZIGZAG[i] as usize] != 0);

                    let mut k = 1usize;
                    let mut zero_run = 0usize;

                    if let Some(last_pos) = last_nonzero_zz {
                        while k <= last_pos {
                            let val = block[ZIGZAG[k] as usize];
                            if val == 0 {
                                zero_run += 1;
                                if zero_run == 16 {
                                    // Emit ZRL.
                                    let (zrl_code, zrl_len) = {
                                        let e = ac_table.encode[0xF0];
                                        if e.1 == 0 {
                                            return Err(DctError::CorruptEntropy);
                                        }
                                        e
                                    };
                                    writer.write_bits(zrl_code, zrl_len);
                                    zero_run = 0;
                                }
                            } else {
                                let (cat, bits, n) = encode_value(val);
                                let rs = ((zero_run as u8) << 4) | cat;
                                let (ac_code, ac_len) = {
                                    let e = ac_table.encode[rs as usize];
                                    if e.1 == 0 {
                                        return Err(DctError::CorruptEntropy);
                                    }
                                    e
                                };
                                writer.write_bits(ac_code, ac_len);
                                writer.write_bits(bits, n);
                                zero_run = 0;
                            }
                            k += 1;
                        }
                    }
                    // Emit EOB only when there are trailing zeros after the last
                    // non-zero coefficient. If the last non-zero is at position 63,
                    // EOB is unnecessary (libjpeg/libjpeg-turbo behaviour).
                    let needs_eob = last_nonzero_zz.map_or(true, |p| p < 63);
                    if needs_eob {
                        let (eob_code, eob_len) = {
                            let e = ac_table.encode[0x00];
                            if e.1 == 0 {
                                return Err(DctError::CorruptEntropy);
                            }
                            e
                        };
                        writer.write_bits(eob_code, eob_len);
                    }
                }
            }
        }

        writer.flush();

        // Reconstruct the full JPEG: everything before entropy data + new
        // entropy data + everything after (from the first post-entropy marker).
        let after_entropy = self.entropy_start + self.entropy_len;
        let mut out = Vec::with_capacity(original.len());
        out.extend_from_slice(&original[..self.entropy_start]);
        out.extend_from_slice(&writer.out);
        out.extend_from_slice(&original[after_entropy..]);
        Ok(out)
    }
}

// ── Magnitude decode helper ───────────────────────────────────────────────────

/// Decode a JPEG magnitude value from its category and raw bits.
fn decode_magnitude(cat: u8, bits: u16) -> i16 {
    if cat == 0 {
        return 0;
    }
    // If the MSB of `bits` is 1, the value is positive; otherwise negative.
    if bits >= (1u16 << (cat - 1)) {
        bits as i16
    } else {
        bits as i16 - (1i16 << cat) + 1
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // Build a minimal valid baseline JPEG from raw pixel data using the
    // `image` crate, so our tests do not depend on external fixture files.
    fn make_jpeg_gray(width: u32, height: u32) -> Vec<u8> {
        use image::{codecs::jpeg::JpegEncoder, GrayImage, ImageEncoder};
        let img = GrayImage::from_fn(width, height, |x, y| {
            image::Luma([(((x * 7 + y * 13) % 200) + 28) as u8])
        });
        let mut buf = Vec::new();
        let enc = JpegEncoder::new_with_quality(&mut buf, 90);
        enc.write_image(img.as_raw(), width, height, image::ExtendedColorType::L8)
            .unwrap();
        buf
    }

    fn make_jpeg_rgb(width: u32, height: u32) -> Vec<u8> {
        use image::{codecs::jpeg::JpegEncoder, ImageEncoder, RgbImage};
        let img = RgbImage::from_fn(width, height, |x, y| {
            image::Rgb([
                ((x * 11 + y * 3) % 200 + 28) as u8,
                ((x * 5 + y * 17) % 200 + 28) as u8,
                ((x * 3 + y * 7) % 200 + 28) as u8,
            ])
        });
        let mut buf = Vec::new();
        let enc = JpegEncoder::new_with_quality(&mut buf, 85);
        enc.write_image(img.as_raw(), width, height, image::ExtendedColorType::Rgb8)
            .unwrap();
        buf
    }

    // ── Error path tests ──────────────────────────────────────────────────────

    #[test]
    fn not_jpeg_returns_error() {
        let result = read_coefficients(b"PNG\x00garbage");
        assert!(matches!(result, Err(DctError::NotJpeg)));
    }

    #[test]
    fn empty_input_returns_error() {
        assert!(matches!(read_coefficients(b""), Err(DctError::NotJpeg)));
    }

    #[test]
    fn truncated_returns_error() {
        // A valid SOI but nothing else.
        assert!(matches!(
            read_coefficients(b"\xFF\xD8\xFF"),
            Err(DctError::Truncated | DctError::Missing(_))
        ));
    }

    #[test]
    fn progressive_jpeg_returns_unsupported() {
        // Craft a minimal JPEG with SOF2 marker.
        let mut data = vec![0xFF, 0xD8]; // SOI
                                         // APP0 JFIF (minimal)
        data.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x10]);
        data.extend_from_slice(&[
            0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00,
        ]);
        // SOF2 marker (progressive)
        data.extend_from_slice(&[0xFF, 0xC2, 0x00, 0x0B]);
        data.extend_from_slice(&[0x08, 0x00, 0x10, 0x00, 0x10, 0x01, 0x01, 0x11, 0x00]);
        let result = read_coefficients(&data);
        assert!(matches!(result, Err(DctError::Unsupported(_))));
    }

    #[test]
    fn incompatible_block_count_returns_error() {
        let jpeg = make_jpeg_gray(16, 16);
        let mut coeffs = read_coefficients(&jpeg).unwrap();
        // Remove one block to make it incompatible.
        coeffs.components[0].blocks.pop();
        let result = write_coefficients(&jpeg, &coeffs);
        assert!(matches!(result, Err(DctError::Incompatible(_))));
    }

    // ── Roundtrip identity tests ──────────────────────────────────────────────

    #[test]
    fn roundtrip_identity_gray() {
        let jpeg = make_jpeg_gray(32, 32);
        let coeffs = read_coefficients(&jpeg).unwrap();
        let reencoded = write_coefficients(&jpeg, &coeffs).unwrap();
        // Re-encoding with unmodified coefficients must produce bit-identical output.
        assert_eq!(jpeg, reencoded, "roundtrip changed the JPEG bytes");
    }

    #[test]
    fn roundtrip_identity_rgb() {
        let jpeg = make_jpeg_rgb(32, 32);
        let coeffs = read_coefficients(&jpeg).unwrap();
        let reencoded = write_coefficients(&jpeg, &coeffs).unwrap();
        assert_eq!(jpeg, reencoded, "roundtrip changed the JPEG bytes");
    }

    #[test]
    fn roundtrip_identity_non_square() {
        let jpeg = make_jpeg_rgb(48, 16);
        let coeffs = read_coefficients(&jpeg).unwrap();
        let reencoded = write_coefficients(&jpeg, &coeffs).unwrap();
        assert_eq!(jpeg, reencoded);
    }

    // ── Modification survival test ────────────────────────────────────────────

    #[test]
    fn lsb_modification_survives_roundtrip() {
        let jpeg = make_jpeg_gray(32, 32);
        let mut coeffs = read_coefficients(&jpeg).unwrap();

        let mut modified_count = 0usize;
        for block in &mut coeffs.components[0].blocks {
            for coeff in block[1..].iter_mut() {
                if coeff.abs() >= 2 {
                    *coeff ^= 1;
                    modified_count += 1;
                }
            }
        }
        assert!(
            modified_count > 0,
            "test image had no eligible coefficients"
        );

        let modified_jpeg = write_coefficients(&jpeg, &coeffs).unwrap();

        // Read back and verify the modifications are preserved.
        let coeffs2 = read_coefficients(&modified_jpeg).unwrap();
        assert_eq!(coeffs.components[0].blocks, coeffs2.components[0].blocks);
    }

    // ── block_count tests ─────────────────────────────────────────────────────

    #[test]
    fn block_count_gray_16x16() {
        let jpeg = make_jpeg_gray(16, 16);
        let counts = block_count(&jpeg).unwrap();
        // 16×16 / 8×8 = 4 blocks for the single Y component.
        assert_eq!(counts, vec![4]);
    }

    #[test]
    fn block_count_rgb_32x32() {
        let jpeg = make_jpeg_rgb(32, 32);
        let counts = block_count(&jpeg).unwrap();
        // For 4:2:0 subsampling: Y has 4×(2×2)=16 blocks, Cb/Cr have 4 each.
        // For 4:4:4: all three have 16 blocks.
        // Accept either — exact layout depends on the encoder.
        assert_eq!(counts.len(), 3);
        let total: usize = counts.iter().sum();
        assert!(total > 0);
    }

    // ── Category function tests ───────────────────────────────────────────────

    #[test]
    fn category_values() {
        assert_eq!(category(0), 0);
        assert_eq!(category(1), 1);
        assert_eq!(category(-1), 1);
        assert_eq!(category(2), 2);
        assert_eq!(category(3), 2);
        assert_eq!(category(4), 3);
        assert_eq!(category(127), 7);
        assert_eq!(category(-128), 8);
        assert_eq!(category(1023), 10);
        assert_eq!(category(i16::MAX), 15); // capped at 15
    }

    // ── Valid output test ─────────────────────────────────────────────────────

    #[test]
    fn output_is_valid_jpeg() {
        let jpeg = make_jpeg_rgb(24, 24);
        let mut coeffs = read_coefficients(&jpeg).unwrap();
        // Flip one LSB.
        if let Some(block) = coeffs.components[0].blocks.first_mut() {
            block[1] |= 1;
        }
        let out = write_coefficients(&jpeg, &coeffs).unwrap();
        // Check SOI and EOI markers.
        assert_eq!(&out[..2], &[0xFF, 0xD8], "missing SOI");
        assert_eq!(&out[out.len() - 2..], &[0xFF, 0xD9], "missing EOI");
    }

    // ── inspect() tests ───────────────────────────────────────────────────────

    #[test]
    fn inspect_gray_returns_correct_dimensions() {
        let jpeg = make_jpeg_gray(32, 16);
        let info = inspect(&jpeg).unwrap();
        assert_eq!(info.width, 32);
        assert_eq!(info.height, 16);
        assert_eq!(info.components.len(), 1);
        assert_eq!(info.components[0].block_count, 8); // 4×2 blocks
    }

    #[test]
    fn inspect_rgb_returns_three_components() {
        let jpeg = make_jpeg_rgb(32, 32);
        let info = inspect(&jpeg).unwrap();
        assert_eq!(info.width, 32);
        assert_eq!(info.height, 32);
        assert_eq!(info.components.len(), 3);
        // Total blocks across components must be positive.
        let total: usize = info.components.iter().map(|c| c.block_count).sum();
        assert!(total > 0);
    }

    #[test]
    fn inspect_matches_block_count() {
        let jpeg = make_jpeg_rgb(48, 32);
        let info = inspect(&jpeg).unwrap();
        let counts = block_count(&jpeg).unwrap();
        let info_counts: Vec<usize> = info.components.iter().map(|c| c.block_count).collect();
        assert_eq!(info_counts, counts);
    }

    // ── eligible_ac_count tests ───────────────────────────────────────────────

    #[test]
    fn eligible_ac_count_is_positive() {
        let jpeg = make_jpeg_rgb(32, 32);
        let n = eligible_ac_count(&jpeg).unwrap();
        assert!(n > 0, "natural image should have eligible AC coefficients");
    }

    #[test]
    fn eligible_ac_count_method_matches_free_fn() {
        let jpeg = make_jpeg_gray(32, 32);
        let coeffs = read_coefficients(&jpeg).unwrap();
        let via_method = coeffs.eligible_ac_count();
        let via_fn = eligible_ac_count(&jpeg).unwrap();
        assert_eq!(via_method, via_fn);
    }

    #[test]
    fn eligible_ac_count_leq_total_ac_count() {
        let jpeg = make_jpeg_rgb(32, 32);
        let coeffs = read_coefficients(&jpeg).unwrap();
        let eligible = coeffs.eligible_ac_count();
        let total_ac: usize = coeffs
            .components
            .iter()
            .flat_map(|c| c.blocks.iter())
            .map(|_| 63) // 63 AC coefficients per block
            .sum();
        assert!(eligible <= total_ac);
    }

    // ── LUT Huffman decode correctness (regression for the old HashMap version) ─

    #[test]
    fn lut_decode_matches_modification_roundtrip() {
        // A natural image exercises many different Huffman code lengths.
        // If the LUT decode is wrong, the modification roundtrip will fail.
        let jpeg = make_jpeg_rgb(64, 64);
        let mut coeffs = read_coefficients(&jpeg).unwrap();
        let mut flipped = 0usize;
        for comp in &mut coeffs.components {
            for block in &mut comp.blocks {
                for coeff in block[1..].iter_mut() {
                    if coeff.abs() >= 2 {
                        *coeff ^= 1;
                        flipped += 1;
                    }
                }
            }
        }
        assert!(flipped > 0);
        let modified = write_coefficients(&jpeg, &coeffs).unwrap();
        let coeffs2 = read_coefficients(&modified).unwrap();
        assert_eq!(coeffs.components.len(), coeffs2.components.len());
        for (c1, c2) in coeffs.components.iter().zip(coeffs2.components.iter()) {
            assert_eq!(c1.blocks, c2.blocks);
        }
    }
}

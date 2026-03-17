# dct-io

Read and write quantized DCT coefficients in baseline JPEG files.

[![Crates.io](https://img.shields.io/crates/v/dct-io)](https://crates.io/crates/dct-io)
[![docs.rs](https://img.shields.io/docsrs/dct-io)](https://docs.rs/dct-io)
[![License](https://img.shields.io/crates/l/dct-io)](LICENSE-MIT)

---

## What it does

JPEG stores images as quantized DCT coefficients encoded with Huffman coding. This crate
parses the entropy-coded stream, decodes the Huffman symbols, and gives you direct access
to the raw quantized coefficient values — without performing IDCT or dequantisation.

You can read coefficients, modify them, and write a new valid JPEG with your changes
re-encoded using the same Huffman tables as the original.

## What it does NOT do

- Decode pixel values (no IDCT, no dequantisation)
- Support progressive JPEG (SOF2), lossless JPEG (SOF3), or arithmetic coding
- Support JPEG 2000

## Supported JPEG variants

- Baseline DCT (SOF0) — the most common variant
- Extended sequential DCT (SOF1)
- Grayscale (1 component) and colour (3 components, typically YCbCr)
- All standard chroma subsampling ratios (4:4:4, 4:2:2, 4:2:0, etc.)
- EXIF and JFIF headers
- Restart markers (DRI / RST0–RST7)

## Usage

```toml
[dependencies]
dct-io = "0.1"
```

### Read and modify coefficients

```rust
use dct_io::{read_coefficients, write_coefficients};

let jpeg = std::fs::read("photo.jpg")?;
let mut coeffs = read_coefficients(&jpeg)?;

// Flip the LSB of every AC coefficient with |v| >= 2 in the luminance channel.
for block in &mut coeffs.components[0].blocks {
    for coeff in block[1..].iter_mut() {
        if coeff.abs() >= 2 {
            *coeff ^= 1;
        }
    }
}

let modified = write_coefficients(&jpeg, &coeffs)?;
std::fs::write("photo_modified.jpg", modified)?;
```

### Inspect image metadata cheaply

```rust
use dct_io::inspect;

let jpeg = std::fs::read("photo.jpg")?;
let info = inspect(&jpeg)?;
println!("{}×{}, {} components", info.width, info.height, info.components.len());
for comp in &info.components {
    println!("  id={} h={} v={} blocks={}", comp.id, comp.h_samp, comp.v_samp, comp.block_count);
}
```

### Count eligible AC positions for steganography

```rust
use dct_io::{read_coefficients, eligible_ac_count};

let jpeg = std::fs::read("photo.jpg")?;

// Cheap path — returns the count directly without allocating coefficient data.
let n = eligible_ac_count(&jpeg)?;
println!("{n} positions available for LSB embedding");

// Or after you already have coefficients:
let coeffs = read_coefficients(&jpeg)?;
println!("{} positions available", coeffs.eligible_ac_count());
```

### Query block counts

```rust
use dct_io::block_count;

let jpeg = std::fs::read("photo.jpg")?;
let counts = block_count(&jpeg)?;
println!("{} components:", counts.len());
for (i, &n) in counts.iter().enumerate() {
    println!("  component {i}: {n} blocks");
}
```

## Coefficient layout

Each `block: [i16; 64]` is in JPEG zigzag scan order:

- Index 0: DC coefficient
- Indices 1–63: AC coefficients in zigzag order

Values are the raw quantized coefficients as stored in the JPEG bitstream. Multiply by
the quantization table to recover the pre-quantized DCT values.

## Safety and correctness

- No `unsafe` code
- No panics on malformed input — all error paths return `DctError`
- Category values clamped to 0–15 to handle out-of-range inputs
- MCU count capped at 1M (prevents excessive allocation on corrupt input)
- Component count validated (max 4)
- Byte stuffing (0xFF → 0xFF 0x00) correctly handled in both directions
- Restart marker (RST0–RST7) handling preserves DC predictor state

## Error handling

```rust
pub enum DctError {
    NotJpeg,              // SOI marker missing
    Truncated,            // file ends mid-stream
    CorruptEntropy,       // invalid Huffman symbol
    Unsupported(String),  // progressive, lossless, arithmetic coding
    Missing(String),      // required marker or table absent
    Incompatible(String), // coefficient data doesn't match the JPEG
}
```

## Limitations

### Roundtrip identity

`write_coefficients(jpeg, read_coefficients(jpeg)?)` produces byte-identical output
for JPEGs encoded by libjpeg, libjpeg-turbo, and most standard encoders. Exotic
encoders that use non-standard Huffman table construction or non-standard EOB placement
may produce a bitstream that decodes identically but is not byte-for-byte equal.

### Multiple scans

Only the first SOS (start-of-scan) is processed. Baseline JPEG has exactly one scan.

## Licence

Licensed under either of:

- [MIT licence](LICENSE-MIT)
- [Apache Licence 2.0](LICENSE-APACHE)

at your option.

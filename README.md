# dct-io

Read and write the hidden numbers inside JPEG files — without touching the pixels.

[![Crates.io](https://img.shields.io/crates/v/dct-io)](https://crates.io/crates/dct-io)
[![docs.rs](https://img.shields.io/docsrs/dct-io)](https://docs.rs/dct-io)
[![License](https://img.shields.io/crates/l/dct-io)](LICENSE-MIT)
[![CI](https://github.com/elementmerc/dct-io/actions/workflows/ci.yml/badge.svg)](https://github.com/elementmerc/dct-io/actions/workflows/ci.yml)

---

## What even is a JPEG?

When you save a photo as a JPEG, your computer doesn't store every pixel's colour directly.
Instead it chops the image into tiny 8×8 pixel tiles and runs a maths trick called the
**Discrete Cosine Transform (DCT)** on each one. Think of it like describing a song with
a list of bass, mid, and treble levels rather than writing out every sound wave. The result
is a list of 64 numbers per tile — the **DCT coefficients** — that capture the image's
frequencies from coarse (the big shapes) to fine (tiny details).

```
Your photo (e.g. 480×320 pixels)
┌──────────────────────────────────────┐
│ ┌──┬──┬──┬──┬──┬──┬──┐              │
│ │  │  │  │  │  │  │  │  ...         │
│ ├──┼──┼──┼──┼──┼──┼──┤              │
│ │  │  │  │  │  │  │  │  ...  each □ │
│ ├──┼──┼──┼──┼──┼──┼──┤  = 8×8 pixels│
│ │  │  │  │  │  │  │  │  ...         │
│ └──┴──┴──┴──┴──┴──┴──┘              │
│         ... more rows ...            │
└──────────────────────────────────────┘
  DCT runs on each tile independently
```

Those numbers are then rounded (this is the "lossy" part) and packed tightly using
**Huffman coding** — a compression trick that assigns shorter codes to more common values.

```
pixels     →   [DCT]   →  coefficients  →  [quantize]  →  integers  →  [Huffman]  →  .jpg
(8×8 tile)              (64 frequencies)   (round down)   (64 ints)                   file
                                ▲                              ▲
                          dct-io reads                  dct-io works
                          and writes here               with these
```

This crate peels back those layers. It reads the compressed data, unpacks the Huffman
codes, and hands you the raw coefficient numbers. You can change them and write a new
JPEG that looks identical to any image viewer but has your modifications baked in at
the compression level.

## What can you do with this?

- **Steganography** — hide data inside a JPEG by tweaking the least significant bit of
  coefficients (the JSteg technique). The image looks the same; the bits are yours.

  ```
  coefficient = 42  →  binary: 0 1 0 1 0 1 [0]
                                            └── you own this bit

  flip to 1:  42 → 43   (invisible to viewers)
  flip to 0:  43 → 42   (reversible — read it back later)

  Rule: only use coefficients where |value| ≥ 2 so flipping
  the LSB never pushes a value through zero (that would change
  the Huffman encoding and corrupt the file).
  ```
- **Watermarking** — embed an invisible signature that survives re-saving.
- **Forensic analysis** — inspect the raw coefficient structure to detect tampering or
  double compression.
- **Research / signal processing** — work directly in the frequency domain without
  decoding to pixels first.

## What this crate does NOT do

- Decode pixel values (no inverse-DCT, no dequantisation — pixels stay out of it)
- Support progressive JPEG, lossless JPEG, or arithmetic coding (returns an error)
- Support JPEG 2000

## Supported JPEG variants

- Baseline DCT (SOF0) — the most common JPEG you'll encounter
- Extended sequential DCT (SOF1)
- Grayscale (1 channel) and colour (3 channels, typically Y/Cb/Cr)
- All standard chroma subsampling ratios (4:4:4, 4:2:2, 4:2:0, etc.)
- EXIF and JFIF headers
- Restart markers (DRI / RST0–RST7)

## Installation

```toml
[dependencies]
dct-io = "0.1"
```

## Examples

### Read and modify coefficients

```rust
use dct_io::{read_coefficients, write_coefficients};

let jpeg = std::fs::read("photo.jpg")?;
let mut coeffs = read_coefficients(&jpeg)?;

// Flip the LSB of every AC coefficient with |v| >= 2 in the Y (luminance) channel.
// These are "eligible" positions — changing them doesn't shift zero runs,
// so the output is a valid JPEG that looks identical to the original.
for block in &mut coeffs.components[0].blocks {
    for coeff in block[1..].iter_mut() {   // index 0 is DC; 1–63 are AC
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
let info = inspect(&jpeg)?;    // does NOT decode the entropy stream
println!("{}×{}, {} components", info.width, info.height, info.components.len());
for comp in &info.components {
    println!("  id={} h={} v={} blocks={}", comp.id, comp.h_samp, comp.v_samp, comp.block_count);
}
```

### Count how many bits you can hide

```rust
use dct_io::{read_coefficients, eligible_ac_count};

let jpeg = std::fs::read("photo.jpg")?;

// Quick path — counts without allocating the full coefficient array:
let n = eligible_ac_count(&jpeg)?;
println!("{n} positions available for LSB embedding ({} bytes)", n / 8);

// Or after you already have coefficients:
let coeffs = read_coefficients(&jpeg)?;
println!("{} positions available", coeffs.eligible_ac_count());
```

### Query block counts

```rust
use dct_io::block_count;

let jpeg = std::fs::read("photo.jpg")?;
let counts = block_count(&jpeg)?;
for (i, &n) in counts.iter().enumerate() {
    println!("component {i}: {n} 8×8 blocks");
}
```

## Coefficient layout

Each `block: [i16; 64]` is in **JPEG zigzag scan order**:

- **Index 0** — the DC coefficient (represents the average brightness/colour of the tile)
- **Indices 1–63** — AC coefficients in zigzag order (higher index = higher frequency detail)

```
8×8 block — coefficient indices in zigzag order

  ◄── low frequency                  high frequency ──►
  ┌────┬────┬────┬────┬────┬────┬────┬────┐
  │  0 │  1 │  5 │  6 │ 14 │ 15 │ 27 │ 28 │  ▲ low
  ├────┼────┼────┼────┼────┼────┼────┼────┤  │ freq
  │  2 │  4 │  7 │ 13 │ 16 │ 26 │ 29 │ 42 │  │
  ├────┼────┼────┼────┼────┼────┼────┼────┤  │
  │  3 │  8 │ 12 │ 17 │ 25 │ 30 │ 41 │ 43 │  │
  ├────┼────┼────┼────┼────┼────┼────┼────┤  │
  │  9 │ 11 │ 18 │ 24 │ 31 │ 40 │ 44 │ 53 │  ▼
  ├────┼────┼────┼────┼────┼────┼────┼────┤
  │ 10 │ 19 │ 23 │ 32 │ 39 │ 45 │ 52 │ 54 │  ▲
  ├────┼────┼────┼────┼────┼────┼────┼────┤  │
  │ 20 │ 22 │ 33 │ 38 │ 46 │ 51 │ 55 │ 60 │  │ high
  ├────┼────┼────┼────┼────┼────┼────┼────┤  │ freq
  │ 21 │ 34 │ 37 │ 47 │ 50 │ 56 │ 59 │ 61 │  │
  ├────┼────┼────┼────┼────┼────┼────┼────┤  │
  │ 35 │ 36 │ 48 │ 49 │ 57 │ 58 │ 62 │ 63 │  ▼
  └────┴────┴────┴────┴────┴────┴────┴────┘
    ↑ block[0] = DC (average brightness of this tile)
    block[1..63] = AC (the detail, from coarse to fine)
```

The values are the **quantized** coefficients exactly as stored in the file. They have
*not* been dequantized; multiply by the quantization table if you want the pre-quantized
DCT values.

The "eligible" positions for safe LSB embedding are AC coefficients with `|v| >= 2`.
Modifying those never changes whether a coefficient is zero or non-zero, so the
Huffman code lengths (and therefore the re-encoded stream structure) stay the same.

## Safety and security

- **`#![forbid(unsafe_code)]`** — zero unsafe Rust in this crate, guaranteed at compile time
- **No panics on crafted input** — every error path returns a `DctError`; the parser
  validates dimensions, sampling factors, Huffman table structure, component indices,
  and MCU counts before touching any data
- **Allocation cap** — MCU count is capped at 1 million (~67 megapixels) to prevent
  memory exhaustion from a malicious input
- **Huffman overflow guard** — canonical code overflow in malformed DHT segments is
  caught before it can write out-of-bounds into the LUT
- **Fuzz targets** included (see `fuzz/`) — run with `cargo fuzz run fuzz_read`

## Error handling

```rust
pub enum DctError {
    NotJpeg,              // doesn't start with a JPEG SOI marker
    Truncated,            // file ends before parsing is complete
    CorruptEntropy,       // invalid or malformed Huffman data
    Unsupported(String),  // progressive, lossless, arithmetic coding, etc.
    Missing(String),      // a required marker or table is absent
    Incompatible(String), // coefficient data doesn't match this JPEG's structure
}
```

All public functions are marked `#[must_use]` — the compiler will warn if you forget
to handle the returned `Result`.

## Limitations

### Roundtrip identity

`write_coefficients(jpeg, read_coefficients(jpeg)?)` produces **byte-identical** output
for JPEGs encoded by libjpeg, libjpeg-turbo, and most standard encoders. Exotic
encoders that use non-standard Huffman table construction or non-standard EOB placement
may decode identically but not round-trip byte-for-byte.

### Multiple scans

Only the first SOS (start-of-scan) segment is processed. Baseline JPEG always has
exactly one scan; progressive JPEG is not supported.

## Fuzzing

```bash
cargo install cargo-fuzz
cargo fuzz run fuzz_read        # throw random bytes at the parser
cargo fuzz run fuzz_roundtrip   # verify read→write→read consistency
```

## Licence

Licensed under either of:

- [MIT licence](LICENSE-MIT)
- [Apache Licence 2.0](LICENSE-APACHE)

at your option.

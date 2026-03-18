# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.1] - 2026-03-18

### Changed
- README: added ASCII diagrams illustrating image tiling, the JPEG compression
  pipeline, the zigzag coefficient layout, and the JSteg LSB embedding rule

## [0.1.0] - 2026-03-18

### Added
- `read_coefficients` — decode quantized DCT coefficients from a baseline JPEG
- `write_coefficients` — re-encode a JPEG with modified coefficients
- `block_count` — query block counts per component without decoding
- `inspect` — read image metadata (dimensions, components) without decoding
- `eligible_ac_count` — count AC coefficients eligible for LSB steganography
- `JpegCoefficients::eligible_ac_count` — same, on an already-decoded result
- LUT-based Huffman decoder (O(1) per symbol, 64 KB table per Huffman table)
- Full support for baseline DCT (SOF0), extended sequential (SOF1), grayscale,
  colour YCbCr, all standard chroma subsampling ratios, EXIF/JFIF headers,
  and restart markers (DRI / RST0–RST7)
- Comprehensive input validation: zero sampling factors, zero image dimensions,
  DHT code overflow, SOS table index out-of-range, DHT class misuse, symbol
  count cap, SOS component count bounds, MCU count overflow protection
- `#![forbid(unsafe_code)]` — no unsafe code in the crate
- `#[must_use]` on all public functions

//! Smoke test — exercises every public function against a synthetically
//! generated JPEG and asserts basic invariants.
//!
//! Run with:  cargo run --example smoke

fn make_jpeg(width: u32, height: u32) -> Vec<u8> {
    use image::{codecs::jpeg::JpegEncoder, GrayImage, ImageEncoder};
    let img = GrayImage::from_fn(width, height, |x, y| {
        image::Luma([((x * 7 + y * 11) % 200 + 28) as u8])
    });
    let mut buf = Vec::new();
    JpegEncoder::new_with_quality(&mut buf, 85)
        .write_image(img.as_raw(), width, height, image::ExtendedColorType::L8)
        .unwrap();
    buf
}

fn main() {
    let jpeg = make_jpeg(32, 16);

    // inspect — no entropy decode
    let info = dct_io::inspect(&jpeg).expect("inspect failed");
    assert_eq!(info.width, 32, "wrong width");
    assert_eq!(info.height, 16, "wrong height");
    assert_eq!(info.components.len(), 1, "expected grayscale (1 component)");

    // block_count
    let counts = dct_io::block_count(&jpeg).expect("block_count failed");
    assert_eq!(counts, vec![8], "expected 4×2 = 8 blocks");

    // read_coefficients
    let coeffs = dct_io::read_coefficients(&jpeg).expect("read_coefficients failed");
    assert_eq!(coeffs.components.len(), 1);
    assert_eq!(coeffs.components[0].blocks.len(), 8);

    // eligible_ac_count (free function and method must agree)
    let via_fn = dct_io::eligible_ac_count(&jpeg).expect("eligible_ac_count failed");
    let via_method = coeffs.eligible_ac_count();
    assert_eq!(via_fn, via_method, "eligible_ac_count mismatch");
    assert!(via_fn > 0, "no eligible AC coefficients");

    // write_coefficients — unmodified round-trip must be byte-identical
    let out = dct_io::write_coefficients(&jpeg, &coeffs).expect("write_coefficients failed");
    assert_eq!(jpeg, out, "round-trip changed the bytes");

    // SOI and EOI markers present in output
    assert_eq!(&out[..2], b"\xff\xd8", "missing SOI");
    assert_eq!(&out[out.len() - 2..], b"\xff\xd9", "missing EOI");

    println!(
        "smoke: ok  ({}×{}px, {} block, {} eligible AC positions)",
        info.width, info.height, counts[0], via_fn
    );
}

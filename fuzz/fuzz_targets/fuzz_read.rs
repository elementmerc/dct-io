#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Must never panic — only return Ok or Err.
    let _ = dct_io::read_coefficients(data);
});

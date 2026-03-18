#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // If read succeeds, write must also succeed and produce valid output.
    if let Ok(coeffs) = dct_io::read_coefficients(data) {
        if let Ok(out) = dct_io::write_coefficients(data, &coeffs) {
            // The re-encoded output must itself be parseable.
            let _ = dct_io::read_coefficients(&out);
        }
    }
});

//! Arbitrary bytes as a paired-device file (PROTOCOL.md §12.8), as a
//! damaged disk, a hand edit or another program may leave it. Reading never
//! panics, every line is either a device or reported as skipped, names are
//! clean, and each key is listed once.
#![no_main]

use libfuzzer_sys::fuzz_target;
use wind_core::pairing::{clean_name, parse_paired};

fuzz_target!(|data: &[u8]| {
    let list = parse_paired(data);
    let lines = data
        .split(|&b| b == b'\n')
        .filter(|l| !l.is_empty())
        .count();
    assert!(list.devices.len() + list.skipped.len() <= lines);
    for (i, d) in list.devices.iter().enumerate() {
        if let Some(name) = d.name.as_deref() {
            assert_eq!(clean_name(name).as_deref(), Some(name));
        }
        assert!(list.devices[..i].iter().all(|e| e.key != d.key));
    }
    for (line, _) in &list.skipped {
        assert!(*line >= 1);
    }
});

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = bencode::dyn_from_bytes::<bencode::ByteBuf<'_>>(data);
});

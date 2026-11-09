//! FrameReader's memory stays within one frame's limit however the bytes
//! are chunked (PROTOCOL.md §4.2: the length is checked before the payload
//! is buffered). Measured with a counting allocator, so this file holds a
//! single test: nothing else allocates while it measures.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};

use wind_core::protocol::*;

struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            let now = LIVE.fetch_add(layout.size(), SeqCst) + layout.size();
            PEAK.fetch_max(now, SeqCst);
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size(), SeqCst);
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

/// Most bytes allocated at once while `f` runs, above what was live before.
fn peak_during<T>(f: impl FnOnce() -> T) -> (T, usize) {
    let base = LIVE.load(SeqCst);
    PEAK.store(base, SeqCst);
    let out = f();
    (out, PEAK.load(SeqCst) - base)
}

const SLACK: usize = 16 * 1024;
const MIB: usize = 1 << 20;

fn hex(s: &str) -> Vec<u8> {
    s.split_whitespace()
        .map(|b| u8::from_str_radix(b, 16).unwrap())
        .collect()
}

#[test]
fn buffering_is_bounded_by_the_frame_limit() {
    // 1. Oversized length and its "payload" in one 8 MiB chunk.
    let mut chunk = hex("50 01");
    chunk.resize(8 * MIB, 0);
    let mut r = FrameReader::new(Channel::Control);
    let (res, peak) = peak_during(|| r.push(&chunk));
    assert_eq!(
        res,
        Err(FrameError::FrameTooLarge {
            len: 4097,
            max: MAX_HELLO_FRAME
        })
    );
    assert!(peak < SLACK, "oversized first frame buffered {peak} B");

    // 2. Length split across pushes, the rest arriving in a 2 MiB chunk.
    let mut r = FrameReader::new(Channel::Clipboard);
    assert_eq!(r.push(&hex("80")), Ok(1));
    let mut chunk = hex("10 00 01");
    chunk.resize(2 * MIB, 0);
    let (res, peak) = peak_during(|| r.push(&chunk));
    assert!(matches!(res, Err(FrameError::FrameTooLarge { .. })));
    assert!(peak < SLACK, "split oversized length buffered {peak} B");

    // 3. A valid frame, then an oversized one, then padding, in one chunk.
    let mut chunk = hex("06 82 02 a1 00 18 2a 80 00 40 01");
    chunk.resize(2 * MIB, 0);
    let mut r = FrameReader::new(Channel::Control);
    let (results, peak) = peak_during(|| {
        let mut rest = &chunk[..];
        let first = r.read(&mut rest).map(|m| m.is_some());
        let second = r.read(&mut rest).map(|m| m.is_some());
        (first, second)
    });
    assert_eq!(results.0, Ok(true));
    assert!(matches!(results.1, Err(FrameError::FrameTooLarge { .. })));
    assert!(peak < SLACK, "frame after a valid one buffered {peak} B");

    // 4. A legal 1 MiB frame in one chunk: the payload buffer holds exactly
    //    the frame, never more.
    let payload_len = MIB;
    let mut payload = hex("82 00 a6 00 01 01 00 02 00 04 00 05 00 06 5a");
    let pad = payload_len - payload.len() - 4;
    payload.extend((pad as u32).to_be_bytes());
    payload.resize(payload_len, 0xab);
    let mut frame = Vec::new();
    varint::encode(payload_len as u64, &mut frame);
    frame.extend(&payload);
    frame.extend(hex("06 82 02 a1 00 18 2a")); // and a next frame after it
    let mut r = FrameReader::new(Channel::Input);
    let (used, peak) = peak_during(|| r.push(&frame));
    assert_eq!(used, Ok(4 + payload_len), "stops at the frame's end");
    assert!(peak <= payload_len + SLACK, "1 MiB frame buffered {peak} B");
    assert!(matches!(
        r.next_message(),
        Ok(Some(Message::Input(InputMessage::PointerMotion(_))))
    ));

    // 5. Many legal frames in one large chunk: one frame at a time.
    let ping = hex("06 82 02 a1 00 18 2a");
    let chunk: Vec<u8> = ping.iter().copied().cycle().take(7 * 300_000).collect();
    let mut r = FrameReader::new(Channel::Control);
    let (count, peak) = peak_during(|| {
        let mut rest = &chunk[..];
        let mut n = 0;
        while r.read(&mut rest).unwrap().is_some() {
            n += 1;
        }
        n
    });
    assert_eq!(count, 300_000);
    assert!(
        peak < SLACK,
        "small frames in a 2 MiB chunk buffered {peak} B"
    );
}

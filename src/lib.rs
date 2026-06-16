pub mod analyzer;
pub mod engine;
pub mod pitch;
pub mod tracker;

use engine::{Engine, OUT_LEN};
use std::ptr::addr_of_mut;

const IN_CAP: usize = 16384;

static mut IN_BUF: [f32; IN_CAP] = [0.0; IN_CAP];
static mut OUT_BUF: [f32; OUT_LEN] = [0.0; OUT_LEN];
static mut ENGINE: Option<Engine> = None;

#[no_mangle]
pub extern "C" fn init(sample_rate: f32) {
    unsafe { *addr_of_mut!(ENGINE) = Some(Engine::new(sample_rate)) };
}

#[no_mangle]
pub extern "C" fn in_ptr() -> *mut f32 {
    addr_of_mut!(IN_BUF) as *mut f32
}

#[no_mangle]
pub extern "C" fn in_cap() -> u32 {
    IN_CAP as u32
}

#[no_mangle]
pub extern "C" fn out_ptr() -> *const f32 {
    addr_of_mut!(OUT_BUF) as *const f32
}

#[no_mangle]
pub extern "C" fn out_len() -> u32 {
    OUT_LEN as u32
}

/// Process `n` samples previously written to `in_ptr()`. Returns the number
/// of new analysis frames; if > 0, fresh results are at `out_ptr()`.
#[no_mangle]
pub extern "C" fn push_samples(n: u32) -> u32 {
    unsafe {
        let engine = (*addr_of_mut!(ENGINE)).as_mut().expect("init not called");
        let n = (n as usize).min(IN_CAP);
        let samples = &(*addr_of_mut!(IN_BUF))[..n];
        let produced = engine.push(samples);
        if produced > 0 {
            engine.write_out(&mut *addr_of_mut!(OUT_BUF));
        }
        produced as u32
    }
}

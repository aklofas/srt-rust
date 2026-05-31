#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use core::mem::MaybeUninit;

use cortex_m_rt::entry;
use cortex_m_semihosting::{debug, hprintln};
use embedded_alloc::Heap;
use panic_semihosting as _;

#[global_allocator]
static HEAP: Heap = Heap::empty();

const HEAP_SIZE: usize = 128 * 1024;
static mut HEAP_MEM: [MaybeUninit<u8>; HEAP_SIZE] = [MaybeUninit::uninit(); HEAP_SIZE];

#[entry]
fn main() -> ! {
    // SAFETY: called once, before any allocation; `&raw mut` avoids a
    // `static_mut_refs` warning.
    unsafe { HEAP.init(core::ptr::addr_of_mut!(HEAP_MEM) as usize, HEAP_SIZE) }

    let mut v: Vec<u8> = Vec::new();
    v.extend_from_slice(&[1, 2, 3]);
    hprintln!("heap ok: {} bytes allocated", v.len());

    debug::exit(debug::EXIT_SUCCESS);
    loop {}
}

#![no_std]
#![no_main]

use cortex_m_rt::entry;
use cortex_m_semihosting::debug;
// Pull in the panic handler (exits with failure on panic).
use panic_semihosting as _;

#[entry]
fn main() -> ! {
    debug::exit(debug::EXIT_SUCCESS);
    loop {}
}

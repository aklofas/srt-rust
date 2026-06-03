# `freertos-srt` — libsrt on bare-metal FreeRTOS

A reference port of **libsrt** (and the MPEG-TS muxer) onto a bare-metal
Cortex-M target running **FreeRTOS + FreeRTOS-Plus-POSIX + lwIP**, demonstrated
as SRT video egress from a microcontroller. Everything builds with the stock
`arm-none-eabi` GCC toolchain and runs under QEMU (`mps2-an386`, Cortex-M4F) —
no hardware required.

> **Most consumers do not need this.** If you are integrating from an
> application runtime, use the language bindings instead — the Python package
> (`tstrans`) or the planned JVM bindings — and point them at a normal OS
> socket. This product is for the narrow case of **linking the C core directly
> into MCU firmware** that has no operating system. It is **not built by
> default**: it is not a Cargo workspace member, and its gates are opt-in.

## Layout

```
freertos-srt/
  build.sh              dispatcher: ./build.sh <target>  ->  firmware.elf
  substrate/            the shared port (compiled by build-common.sh)
    build-common.sh       parameterized build (LWIP/LIBSRT/NETIF/ENCRYPT knobs)
    arm-none-eabi.cmake   cross toolchain for the vendored libsrt build
    startup.c mps2_an386.ld  reset vector + linker script
    clock_shim.c          hi-res clock_gettime/gettimeofday over SysTick
    atomic64_stub.c syscalls_stub.c net_shim.c   newlib/lwIP impedance shims
    cxa_override.cpp      per-task C++ exception state in FreeRTOS TLS
    pthread_key_shim.c    pthread TSD over a FreeRTOS TLS slot
    srt_opts.h            shared SRT socket setup (transtype/buffers/passphrase)
    freertos/  lwip/  posix-shims/  drivers/  mbedtls/  patches/
  example/              the flagship: SRT egress out a real NIC to a host
    main.cpp  README.md  host/   (a tst-srt listener that verifies byte-exact)
  tests/               five layered verifications, smallest first
    exceptions/        concurrent per-task C++ throw/catch
    lwip-loopback/     564B golden through a lwIP UDP loopback socket
    libsrt-smoke/      srt_startup + create_socket + close + cleanup
    loopback-arq/      SRT byte-exact recovery under ~20% loss (plain + AES)
  build/               all generated output (gitignored; ./build.sh clean wipes it)
```

The `substrate/` is built once and shared. Each target flips a few knobs
(`LWIP`, `LIBSRT`, `NETIF`, `ENCRYPT`) that `build.sh` sets per target; the
superset config compiles both the Ethernet/ARP path and the built-in loopback
path, with `#ifndef`-guarded toggles overridden per target via `-D`.

## Build and run

```bash
# from this directory
./build.sh <exceptions|lwip-loopback|libsrt-smoke|loopback-arq|example>
# ENCRYPT=1 selects the mbedTLS AES-128 build for loopback-arq / example
ENCRYPT=1 ./build.sh loopback-arq
```

`build.sh` emits `build/firmware.elf` (all generated output lives under `build/`;
`./build.sh clean` removes it). Run it under QEMU, e.g.:

```bash
qemu-system-arm -machine mps2-an386 -nographic \
  -semihosting-config enable=on,target=native -kernel build/firmware.elf
```

The first `libsrt`/`example`/`loopback-arq` build cross-compiles the vendored
libsrt (and, with `ENCRYPT=1`, mbedTLS) for the target — a few minutes; warm
builds are seconds. Prerequisites: `arm-none-eabi-gcc`/`g++`, `qemu-system-arm`,
`cmake`, and (for the example host) `cargo`.

## Production crypto warning

> **The AES-128 path in this reference uses _deterministic_ entropy.** The
> `ENCRYPT=1` builds wire mbedTLS, but the entropy hooks in
> `substrate/syscalls_stub.c` (`_getentropy` and `mbedtls_hardware_poll`) are a
> fixed-seed LCG, chosen so the QEMU/CI gate reproduces bit-for-bit. **This is
> not cryptographically secure.** Before enabling SRT encryption in production
> firmware, replace both hooks with a hardware RNG or your board's approved
> entropy source — otherwise the key material is predictable.

## The gate

One dispatcher gate builds a target, runs it under QEMU, and asserts its PASS
token:

```bash
bash scripts/check/embedded/freertos-srt.sh <target>   # from the workspace root
```

It skips cleanly when the cross-toolchain / QEMU / cmake / cargo is absent. The
first four targets are CI hard-gates; the NIC-egress `example` (a networked,
two-process gate) is phase-in.

## The five tests, one line each

| target | proves |
|---|---|
| `exceptions` | concurrent per-task C++ exceptions are isolated on FreeRTOS + the pthread backend |
| `lwip-loopback` | the FreeRTOS + lwIP + hi-res-clock substrate round-trips the golden over a UDP loopback socket |
| `libsrt-smoke` | the cross-compiled libsrt boots its runtime (startup → socket → cleanup) on the substrate |
| `loopback-arq` | SRT recovers the golden byte-exact under ~20% packet loss, plain and AES-128 |
| `example` | a real-NIC SRT caller streams the golden to a host listener byte-exact, plain and AES-128 |

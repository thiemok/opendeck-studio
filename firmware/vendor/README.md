# Vendored crates

## xtensa-lx-rt 0.23.0 (patched)

**Why:** with xtensa-lx-rt 0.23.0, every esp-hal app on the ESP32 hung before `main`. The board then reset through the RTC watchdog about every 8 s (`rst:0x10 (RTCWDT_RTC_RESET)` right after `boot: Loaded app from partition`). Found on 2026-11-15 with the Xtensa toolchains v1.100.0.0 and v1.99.0.0. esp-hal 1.2.2 and 1.0.0 were both affected, as was the plain `esp-generate` template.

**Cause:** the crate writes its `Reset` entry routine as three separate `global_asm!` blocks:

1. stack, `__pre_init`, `.bss` and `.data`
2. the `CCOMPARE` timer reset
3. `vecbase`, `__post_init` and `main`

The routine only works if the blocks end up back to back in `.rwtext`. Recent toolchains place them independently. In our image the tail landed before `Reset`, so the CPU ran off the end of block 1 into padding, hit an illegal instruction before `vecbase` was set, and spun in the ROM's exception vector.

**Patch:** `src/lib.rs` emits the routine as one `cfg_global_asm!` block, so it can't be split. Nothing else differs from the crates.io release, apart from removing `.cargo_vcs_info.json`, `Cargo.toml.orig` and `Cargo.lock`. `firmware/Cargo.toml` points at this copy through `[patch.crates-io]`.

**How it was found:** single-character UART writes (`0x3FF4_0000`) between the steps of `Reset` printed `1234` and stopped. The disassembly then showed `Reset` ending at `memw`, with `wsr.vecbase` placed at a lower address.

**Removing it:** delete this directory and the `[patch.crates-io]` entry once an upstream xtensa-lx-rt release keeps `Reset` in one block. Check with `xtensa-esp32-elf-objdump -d` that `wsr.vecbase` follows `memw` inside `<Reset>`, then confirm the boot on hardware.

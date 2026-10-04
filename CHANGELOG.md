# Changelog

What somebody who pinned an earlier prebuild has to know before pinning the next one. The
generated release notes list the pull requests; this file answers the questions a list of pull
requests does not: **does my C caller still compile, and does the same picture still come out
the same?**

```text
ABI     the C header. Only grows: fields at the end of rimg_options, new numbers for new formats
        and status codes. RIMG_ABI_VERSION moves only when that promise is broken, which is not
        planned.
output  what a given input turns into: which pictures are refused, and the bytes of the ones
        that are not. A decoder or encoder upgrade may move both, and a caller that stores a
        hash of the result has to know.
build   what the prebuild archive holds and what the caller's link line needs.
```

## 0.1.1

Not released yet.

- **ABI** unchanged.
- **output** unchanged; the same image-rs.
- **build** Five more targets: `x86_64-` and `aarch64-unknown-linux-musl` for Alpine,
  `x86_64-pc-windows-gnu` for mingw-w64's gcc, `x86_64-` and `aarch64-pc-windows-gnullvm` for zig
  and llvm-mingw. The MinGW archives hold `librust_image_ffi.a`, the staticlib, as MSVC's hold the
  `.lib`. **A zig build for Windows takes `-gnullvm`.**
  The objects are a fifth of the size they were (2.3 MB instead of 12 MB on x86_64 Linux): the
  release build now makes the staticlib alone, and fat LTO applies to it. Same interface, same
  link line.
  Releases build with `--locked` and only when `cargo audit` finds no known vulnerability in a
  dependency.

## 0.1.0

The first release.

- **ABI** `RIMG_ABI_VERSION` 1. `rimg_reencode`, `rimg_probe`, `rimg_options_default`,
  `rimg_status_string`, `rimg_abi_version`.
- **output** JPEG, PNG and WebP in -- JPEG only unless the caller allows more --, JPEG or PNG
  out. image-rs 0.25.10, pinned exactly.
- **build** One archive per target: the object (`rust_image_ffi.o`; `rust_image_ffi.lib` on
  Windows, where MSVC has no partial link), `rust_image_ffi.h`, `NATIVE_LIBS.txt` with what the
  link line needs, and `SHA256SUMS`. Linux (glibc), macOS and Windows, x64 and arm64. On macOS
  the object exports one symbol besides `rimg_*`: `_rust_eh_personality`, weak -- for the reason
  written down in `scripts/localize.sh`.

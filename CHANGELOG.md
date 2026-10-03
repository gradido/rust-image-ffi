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

## 0.1.0

Not released yet.

- **ABI** `RIMG_ABI_VERSION` 1. `rimg_reencode`, `rimg_probe`, `rimg_options_default`,
  `rimg_status_string`, `rimg_abi_version`.
- **output** JPEG, PNG and WebP in -- JPEG only unless the caller allows more --, JPEG or PNG
  out. image-rs 0.25.10, pinned exactly.
- **build** One archive per target: the object (`rust_image_ffi.o`; `rust_image_ffi.lib` on
  Windows, where MSVC has no partial link), `rust_image_ffi.h`, `NATIVE_LIBS.txt` with what the
  link line needs, and `SHA256SUMS`. Linux (glibc), macOS and Windows, x64 and arm64. On macOS
  the object exports one symbol besides `rimg_*`: `_rust_eh_personality`, weak -- for the reason
  written down in `scripts/localize.sh`.

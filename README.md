# rust-image-ffi

[image-rs](https://github.com/image-rs/image) behind a C interface, as a prebuilt object.

One job: take a picture nobody vouches for, **decode it under hard limits and encode the pixels
again**. What comes out was written by this module's encoder from pixels alone, so nothing of the
input's container survives -- no EXIF, ICC profile, comment or text chunk, no bytes behind the end
marker, no second file hiding in the first. A picture that does not decode is refused.

First user: [gradido](https://github.com/gradido/gradido), where avatars and the pictures in chat
messages are stored by the server and shown to many members' browsers. Today the server checks the
first and last two bytes of a JPEG (`core/src/logic/JpegImage.logic.ts`); with this module in
`shared-native` it stores only what it encoded itself.

It holds mechanism and no policy. Which formats come in, how large a picture may be and how many
bytes it may cost are the caller's.

## The interface

`include/rust_image_ffi.h` is the one file a C caller reads. In short:

```c
rimg_options opt;
rimg_options_default(&opt);            /* JPEG in, JPEG out, 8192 x 8192, 16 MP, quality 85 */
opt.max_width = opt.max_height = 4096;
opt.max_pixels = 500000;

uint8_t out[35 * 1024];                /* the byte budget is the size of the buffer */
size_t out_len;
rimg_info info;
int32_t status = rimg_reencode(&opt, in, in_len, out, sizeof(out), &out_len, &info);
/* RIMG_OK: out[0..out_len] is what gets stored, info.width/height are what it really is.
 * RIMG_ERR_BUFFER_TOO_SMALL: over budget at this quality; out_len says by how much.
 * RIMG_ERR_UNSUPPORTED, _DECODE, _LIMIT: refused. rimg_status_string(status) for the log. */
```

No handle, no state, no buffer the caller frees. Every function is thread-safe; a panic is caught
at the boundary and becomes `RIMG_ERR_PANIC`. `rimg_reencode` is CPU work for the length of the
call -- a Node addon calls it from a worker (`Napi::AsyncWorker`), not on the event loop.

What it does, in this order:

1. The format is decided on the first bytes, never on a name or a content type, and has to be in
   `input_formats`. The default is JPEG only: every format allowed is one more decoder that reads
   hostile bytes. PNG and WebP are compiled in and off until asked for.
2. The header is read, and width, height, pixel count and the memory decoding would take are
   checked against the options -- before a pixel is decoded. A few bytes that announce
   60000 x 60000 are refused here.
3. The pixels are decoded. Anything the decoder does not accept is `RIMG_ERR_DECODE`.
4. The EXIF orientation is applied to the pixels (the tag itself is gone afterwards, so without
   this an upright photo comes out on its side). Transparency is laid over a background color for
   JPEG and kept for PNG. 16 bit becomes 8.
5. The pixels are encoded, as JPEG or PNG.

## What it does not protect against

- **What the pixels show.** A picture of something unwanted is still that picture.
- **Bugs in the decoders.** They are safe Rust, which rules out the memory corruption that image
  libraries in C are known for, and the limits bound time and memory -- but a decoder that takes
  long on a crafted file within the limits is possible. The limits are the caller's lever.
  Fuzzing (`compare/stb`) found crafted JPEGs on which zune-jpeg 0.5.15 panics: the call answers
  `RIMG_ERR_PANIC`, the picture is refused, and Rust's panic message goes to stderr.
- **Color accuracy.** The ICC profile is dropped with everything else and not applied first, so a
  wide-gamut photo (Display P3 from a phone) comes out slightly less saturated in a browser.
  Applying it would mean parsing one more untrusted structure.
- **Generation loss.** JPEG to JPEG costs quality every time; re-encode once, where the picture
  enters, and store the result.

Of an animated PNG or WebP only the first frame is taken. There is no scaling.

## Layout

```text
include/rust_image_ffi.h    the interface
src/ffi.rs                  the extern "C" functions; the only module allowed unsafe
src/reencode.rs             sniff, limits, decode, orientation, encode -- safe Rust over slices
src/abi.rs                  the C types, field for field, and the defaults
scripts/localize.sh         release build -> dist/<target>/ the object, .h, NATIVE_LIBS.txt, SHA256SUMS
scripts/c-smoke.sh          links tests/c/smoke.c against that object with cc and zig cc
scripts/release-version.sh  what makes a merge a release
tests/reencode.rs           through the C interface: payloads in segments, chunks and behind the
                            end marker, orientation, formats, limits, a decompression bomb,
                            buffers, option structs of an older size
tests/abi_layout.rs         the C compiler's layout of the header against the Rust one
examples/make_fixture.rs    writes tests/c/fixture.h, the JPEG the C smoke test feeds in
compare/stb/                the same job with stb_image, for size and fuzzing; see its README
compare/stb-wasm/           that stb build as WebAssembly under Node; see its README
```

## Build and test

```sh
cargo test                  # needs a C compiler for the layout test
scripts/localize.sh         # dist/host/rust_image_ffi.o
scripts/c-smoke.sh          # the shipped object, linked from C and run
```

The toolchain is pinned in `rust-toolchain.toml` and image-rs to an exact version in `Cargo.toml`:
the decoders are the attack surface, so an upgrade is a deliberate release, not a lockfile update.

## What ships: a localized object

As in [libp2p-ffi](https://github.com/gradido/libp2p-ffi), whose scripts these are: a caller links
`rust_image_ffi.o`, a single relocatable object in which only the `rimg_` functions are global, so
that it can sit in one binary with another Rust staticlib -- libp2p-ffi, for one -- without the two
colliding on `rust_eh_personality` or on an allocator. `scripts/localize.sh` carries the reasons at
the lines they apply to, including the two exceptions: Windows ships the staticlib because MSVC has
no partial link, and macOS exports `_rust_eh_personality` as a weak symbol.

`NATIVE_LIBS.txt` is what the caller's link line needs beside the object, printed by rustc; on
Linux `-lgcc_s -lutil -lrt -lpthread -lm -ldl -lc`. `panic` stays `"unwind"` in the release
profile, or a panic would end the host instead of answering `RIMG_ERR_PANIC`.

Size, measured on x86_64 Linux: the object is 12 MB of per-function sections; `tests/c/smoke.c`
linked against it with `--gc-sections` and stripped is 1.1 MB.

## Using it from gradido's shared-native

Not done here; what the build there has to settle:

- **Fetching.** `shared-native` compiles everything with zig and has no prebuilds so far. This is
  the first: `build_helper` downloads `rust-image-ffi-<version>-<target>.tar.gz` from the release,
  checks it against a pinned SHA-256, and `build_napi.zig` adds the object with `addObjectFile`
  and the libraries from `NATIVE_LIBS.txt`. zig links it -- `scripts/c-smoke.sh` does exactly that
  with `zig cc`, which needs `-lunwind` beside rustc's list.
- **Targets that are missing.** `detectTargetTriple` there also answers musl, 32-bit x86 and arm,
  and on Windows zig builds for the gnu ABI while the prebuild is MSVC's. The release matrix here
  is glibc Linux, macOS and MSVC Windows, x64 and arm64. Linux in Docker (bookworm) and macOS are
  covered; Windows and Alpine need a row each (`*-pc-windows-gnullvm`, `*-unknown-linux-musl`)
  before the build there can rely on it.
- **The byte budgets.** `CHAT_IMAGE_MAX_BYTES` and `AVATAR_*_MAX_BYTES` bound what the browser
  sends. The re-encoded picture is a different size -- larger, when the browser encoded below the
  quality asked for here. Passing the budget as `out_cap` makes that an answer
  (`RIMG_ERR_BUFFER_TOO_SMALL`) the caller can retry at a lower `jpeg_quality` or refuse.
- **Width and height.** A chat picture's size is what the sender says today. `rimg_info` says what
  it is.

## Releases

The same rule as libp2p-ffi: a release is a pull request whose **title says "release"** and whose
**`Cargo.toml` version has not been released before**. `scripts/release-version.sh` is that rule,
`.github/workflows/release-gate.yml` runs it on the open pull request and `prebuild.yml` again at
merge, then builds, tests and smoke-links every target on a native runner and publishes one archive
per target with a `SHA256SUMS` over them. Make the check named *release version* required on the
default branch. [`CHANGELOG.md`](CHANGELOG.md) says per version what moved in the ABI, in the
output and in the build.

## License

Apache-2.0

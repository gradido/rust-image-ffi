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

## How large the result is

Re-encoding does not make a picture larger by itself; the settings do. Measured on an 800 x 600
picture that came in as a 26.4 KB JPEG of quality 60 with color at half resolution (4:2:0), which
is what browsers and cameras write:

```text
jpeg_quality    0.1.1, 4:2:0 (default)    0.1.1, 4:4:4    0.1.0     libjpeg, 4:2:0, optimized
85              33.1 KB                   53.9 KB         61.7 KB   33.4 KB
75              30.6 KB                   44.0 KB         49.3 KB   30.7 KB
60              26.4 KB                   35.3 KB         40.8 KB   26.4 KB
50              25.6 KB                   32.4 KB         38.2 KB   25.5 KB
```

- **At the source's quality the picture keeps its size**: 26.4 KB in, 26.4 KB out, and the same
  again on a second pass.
- **A quality above the source's buys nothing**: 85 for a picture that was stored at 60 keeps its
  artifacts more precisely, at a quarter more bytes. A caller that knows what its clients send
  asks for that quality.
- **0.1.0 was half again as large.** It encoded with image-rs's own encoder, which stores color at
  full resolution and uses the standard Huffman tables. Since 0.1.1 the JPEG encoder is
  [mozjpeg](https://github.com/mozilla/mozjpeg), Mozilla's fork of libjpeg-turbo, through the
  `mozjpeg` crate -- set to do what libjpeg-turbo does, which is why the last column matches:
  a baseline JPEG in one interleaved scan, 4:2:0 by default (`jpeg_subsampling = 0` for full
  resolution), Huffman tables built for the picture. mozjpeg's own defaults, progressive scans and
  trellis quantization, are left off.
- **Why C in a module that is about memory safety.** An encoder only ever sees pixels this module
  decoded, never the sender's bytes; what reads hostile input is still Rust alone. And the
  pure-Rust encoder that was tried first, `jpeg-encoder`, can build its Huffman tables only by
  writing one scan per component -- a file that zune-jpeg 0.5.15, the decoder this module reads
  JPEGs with, gets the colors of wrong, where libjpeg, ffmpeg and stb read it right
  (`compare/stb/findings/zune-miscolors-noninterleaved-420.jpg` is one). A picture goes through
  this module on the sending server and again on the receiving one, so it has to read what it
  writes; `tests/reencode.rs` holds it to that.

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
scripts/audit.sh            Cargo.lock against the RustSec advisory database
tests/reencode.rs           through the C interface: payloads in segments, chunks and behind the
                            end marker, orientation, formats, limits, a decompression bomb,
                            buffers, option structs of an older size
tests/abi_layout.rs         the C compiler's layout of the header against the Rust one
examples/make_fixture.rs    writes tests/c/fixture.h, the JPEG the C smoke test feeds in
fuzz/                       libFuzzer with AddressSanitizer over the C interface; needs nightly
compare/stb/                the same job with stb_image, for size and fuzzing; see its README
compare/stb-wasm/           that stb build as WebAssembly under Node; see its README
```

## Build and test

```sh
cargo test                  # needs a C compiler for the layout test
scripts/localize.sh         # dist/host/rust_image_ffi.o
scripts/c-smoke.sh          # the shipped object, linked from C and run
scripts/audit.sh            # known vulnerabilities in the dependencies
fuzz/run.sh                 # 30 minutes of fuzzing with AddressSanitizer; needs nightly
```

The toolchain is pinned in `rust-toolchain.toml` and image-rs to an exact version in `Cargo.toml`:
the decoders are the attack surface, so an upgrade is a deliberate release, not a lockfile update.
Every crate behind it is pinned by `Cargo.lock`, and the release build and the tests in CI run with
`--locked`: a lockfile that does not match is a failed build, not a silent update.
`.github/workflows/audit.yml` runs `scripts/audit.sh` on every pull request and once a week, and a
release does not build while it fails.

`fuzz/run.sh` is the check before a release that changes a decoder: every format allowed, both
encoders, through `rimg_reencode` and `rimg_probe`, with AddressSanitizer watching the `unsafe`
that the decoders' SIMD code needs. It is the one thing here that needs a nightly compiler, and
it is not part of CI: a run is half an hour on eight cores. With a clang at hand the C encoder is
built with AddressSanitizer too. The last runs, on 2026-10-04 on 28 workers, without a finding:
34 million inputs in half an hour against 0.1.0's encoder, and 4.6 million new inputs in ten
minutes after the change to mozjpeg.

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

Size, measured on x86_64 Linux: the object is 2.7 MB; `tests/c/smoke.c` linked against it with
`--gc-sections` and stripped is 1.2 MB, of which the C encoder is 0.3 MB. (Release 0.1.0 shipped
12 MB: it built the crate's other crate types along with the staticlib, and that kept fat LTO from
applying to it.)

The C in it, the encoder, is compiled with zig for every target except MSVC's -- the compiler
gradido's shared-native builds its own C with, the same on every runner, with no toolchain to
install per target. `scripts/localize.sh` takes it from PATH, from where shared-native keeps its
own, or from `pip install ziglang`. The `*-windows-msvc` archives are for callers on MSVC's
toolchain and are built with `cl`. Without zig, a build for this machine falls back to `cc` and
says so; a release does not.

What a release holds, one archive per target:

```text
x86_64-unknown-linux-gnu     aarch64-unknown-linux-gnu      rust_image_ffi.o
x86_64-unknown-linux-musl    aarch64-unknown-linux-musl     rust_image_ffi.o     Alpine
x86_64-apple-darwin          aarch64-apple-darwin           rust_image_ffi.o
x86_64-pc-windows-msvc       aarch64-pc-windows-msvc        rust_image_ffi.lib
x86_64-pc-windows-gnu                                       librust_image_ffi.a  mingw-w64 gcc
x86_64-pc-windows-gnullvm    aarch64-pc-windows-gnullvm     librust_image_ffi.a  zig, llvm-mingw
```

The musl object is built for a caller that links musl dynamically, as a Node addon on Alpine
does: it asks for `-lgcc_s -lc` and carries no libc of its own. The MinGW targets ship the
staticlib like MSVC does. `-gnu` and `-gnullvm` differ in the unwinder they expect -- libgcc's or
LLVM's libunwind -- and zig brings the second, so **a zig build for Windows takes `-gnullvm`**.

## Using it from gradido's shared-native

Not done here; what the build there has to settle:

- **Fetching.** `shared-native` compiles everything with zig and has no prebuilds so far. This is
  the first: `build_helper` downloads `rust-image-ffi-<version>-<target>.tar.gz` from the release,
  checks it against a pinned SHA-256, and `build_napi.zig` adds the object with `addObjectFile`
  and the libraries from `NATIVE_LIBS.txt`. zig links it -- `scripts/c-smoke.sh` does exactly that
  with `zig cc`, which needs `-lunwind` beside rustc's list.
- **Which archive.** zig's target says it: `*-linux-gnu` and `*-linux-musl` take the Rust target
  of the same name, `*-macos` takes `*-apple-darwin`, and `*-windows` -- where zig builds for the
  gnu ABI -- takes `*-pc-windows-gnullvm`, not MSVC's. `detectTargetTriple` there also answers
  32-bit x86 and arm; for those there is no prebuild.
- **The byte budgets.** `CHAT_IMAGE_MAX_BYTES` and `AVATAR_*_MAX_BYTES` bound what the browser
  sends. The re-encoded picture is a different size; see *How large the result is*. Passing the
  budget as `out_cap` makes that an answer (`RIMG_ERR_BUFFER_TOO_SMALL`) the caller can retry at
  a lower `jpeg_quality` or refuse.
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

Apache-2.0. The JPEG encoder it links, mozjpeg, is under the IJG license, the BSD 3-clause
license of libjpeg-turbo and the zlib license: this software is based in part on the work of the
Independent JPEG Group.

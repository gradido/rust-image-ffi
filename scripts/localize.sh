#!/bin/sh
# Builds the release staticlib and turns it into what this module ships, per object format:
#
#   ELF, Mach-O   one relocatable object in which only the rimg_ functions are global
#   COFF          the staticlib itself; see *Windows* below
#
# ELF covers glibc and musl. A musl build is for a caller that links musl dynamically -- a Node
# addon on Alpine is a shared object --, so it is built without crt-static and asks for libgcc_s,
# which every Alpine with a C++ runtime has, instead of a static libunwind, which none has by
# default. The archive carries no libc of its own either way.
#
# Why localize: two Rust staticlibs built with different rustc versions collide on
# rust_eh_personality when linked into one binary, and a #[global_allocator] in one of them fails
# the link or silently takes over the other's allocations. A localized object has neither problem.
#
#   scripts/localize.sh [target-triple]
#       -> dist/<triple or host>/{rust_image_ffi.o | rust_image_ffi.lib | librust_image_ffi.a},
#          rust_image_ffi.h, NATIVE_LIBS.txt, SHA256SUMS
#
# Windows: MSVC's toolchain has no partial link -- neither link.exe nor lld-link takes -r -- so
# there is nothing to localize into. The .lib ships as it is. The clash localization avoids is
# ELF's: a COMDAT group named DW.ref.rust_eh_personality. COFF has no counterpart of it, and two
# staticlibs that carry std twice are folded by COMDAT selection. Untested with a second Rust
# staticlib in one binary, and recorded as such in README.md.
#
# MinGW (*-windows-gnu for mingw-w64's gcc, *-windows-gnullvm for zig and llvm-mingw) ships the
# staticlib as well, as librust_image_ffi.a, for the same reason and with the same caveat. GNU ld
# could link a PE object partially; nobody has tried what the result does to Rust's unwind tables.
# These targets are built from any host -- a staticlib needs no linker -- which is why the format
# below follows the target and not the machine.
set -eu

target=${1:-}
root=$(cd "$(dirname "$0")/.." && pwd)

# By the target when one is named, by this machine otherwise.
case "$target" in
    *-linux-*) format=elf ;;
    *-apple-*) format=macho ;;
    *-windows-*) format=coff ;;
    "")
        case "$(uname -s)" in
            Linux*) format=elf ;;
            Darwin*) format=macho ;;
            MINGW* | MSYS* | CYGWIN*) format=coff ;;
            *) echo "unsupported host $(uname -s)" >&2; exit 1 ;;
        esac
        ;;
    *) echo "unsupported target $target" >&2; exit 1 ;;
esac

case "$target" in
    *-linux-musl) export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-C target-feature=-crt-static" ;;
esac

# The JPEG encoder is C, and it is built with zig for every target but MSVC's: the same compiler
# on every runner, the one gradido's shared-native builds its own C with, and one that needs no
# toolchain installed per target. zig is taken from PATH, from where shared-native keeps its own,
# or from pip. The *-windows-msvc artifacts are for callers on MSVC's toolchain and are built
# with it -- cl, which the cc crate finds by itself.
#
# An explicit CC_<target> in the environment wins. Without zig, a build for this machine falls
# back to the system's cc and says so; RIMG_REQUIRE_ZIG=1, which the release workflow sets, makes
# that an error, so that a failed installation cannot change what a release is built with.
zig=""
if command -v zig > /dev/null 2>&1; then
    zig="zig"
elif [ -x "$HOME/.zig-build/zig/0.15.2/zig" ]; then
    zig="$HOME/.zig-build/zig/0.15.2/zig"
elif python3 -m ziglang version > /dev/null 2>&1; then
    zig="python3 -m ziglang"
elif python -m ziglang version > /dev/null 2>&1; then
    zig="python -m ziglang"
fi

triple=${target:-$(cd "$root" && rustc -vV | sed -n 's/^host: //p')}
arch=${triple%%-*}
zig_target=""
fallback=0
case "$triple" in
    # glibc 2.17 is what Rust's own standard library asks for; the C must not ask for more.
    *-linux-gnu) zig_target="$arch-linux-gnu.2.17"; [ -z "$target" ] && fallback=1 ;;
    *-linux-musl) zig_target="$arch-linux-musl" ;;
    # The oldest macOS Rust itself builds for, so that the linker does not find objects in one
    # file that disagree about it.
    x86_64-apple-darwin) zig_target="x86_64-macos.10.12"; fallback=1 ;;
    aarch64-apple-darwin) zig_target="aarch64-macos.11.0"; fallback=1 ;;
    *-windows-gnu | *-windows-gnullvm) zig_target="$arch-windows-gnu" ;;
esac
wrappers=""
if [ -n "$zig_target" ]; then
    env_target=$(printf '%s' "$triple" | tr '-' '_')
    if eval "[ -z \"\${CC_$env_target:-}\" ]"; then
        if [ -z "$zig" ] && [ "$fallback" = 1 ] && [ "${RIMG_REQUIRE_ZIG:-0}" != 1 ]; then
            echo "note: no zig found; the C encoder is built with the system's cc" >&2
            zig_target=""
        elif [ -z "$zig" ]; then
            echo "building for $triple needs zig as its C compiler: install it (pip install" \
                "ziglang), or set CC_$env_target" >&2
            exit 1
        fi
    fi
    if [ -n "$zig_target" ] && eval "[ -z \"\${CC_$env_target:-}\" ]"; then
        # Through a wrapper, for one reason: the cc crate sees a clang and hands it
        # --target=<Rust's triple>, a spelling zig does not know. The wrapper drops it; the
        # target zig is to build for is the one named here.
        wrappers=$(mktemp -d)
        cat > "$wrappers/cc" << EOF
#!/bin/sh
for arg do
    shift
    case "\$arg" in --target=*) ;; *) set -- "\$@" "\$arg" ;; esac
done
exec $zig cc -target $zig_target "\$@"
EOF
        printf '#!/bin/sh\nexec %s ar "$@"\n' "$zig" > "$wrappers/ar"
        chmod +x "$wrappers/cc" "$wrappers/ar"
        case "$(uname -s)" in
            # Windows starts programs, not scripts: the shell is named with it.
            MINGW* | MSYS* | CYGWIN*) run_script="sh " ;;
            *) run_script="" ;;
        esac
        export "CC_$env_target=$run_script$wrappers/cc"
        export "AR_$env_target=$run_script$wrappers/ar"
    fi
fi

sums() {
    if command -v sha256sum > /dev/null 2>&1; then sha256sum "$@"; else shasum -a 256 "$@"; fi
}

# --locked: exactly the versions in Cargo.lock, or no build. The decoders are what this module is
# trusted for, and a release must not pick up one that nobody looked at.
#
# Only the staticlib: the crate is also a cdylib, and linking that needs the target's linker,
# which a build for another system does not have and this script has no use for.
cargo rustc --locked --release --lib --crate-type staticlib --manifest-path "$root/Cargo.toml" \
    ${target:+--target "$target"}
base="${CARGO_TARGET_DIR:-$root/target}/${target:+$target/}release"
out="$root/dist/${target:-host}"
mkdir -p "$out"

work=$(mktemp -d)
trap 'rm -rf "$work" ${wrappers:+"$wrappers"}' EXIT

# What the caller has to put on its link line beside this object. Printed by rustc rather than
# written down here, because it differs per platform and moves with the dependencies.
cargo rustc --locked --release --lib --crate-type staticlib --quiet --manifest-path "$root/Cargo.toml" \
    ${target:+--target "$target"} -- --print native-static-libs 2>&1 | sed -n 's/^note: native-static-libs: *//p' | head -1 \
    > "$out/NATIVE_LIBS.txt"

case "$format" in
elf)
    lib="$base/librust_image_ffi.a"
    # Members of a Rust staticlib may share a name, and a plain `ar x` would let the last one
    # overwrite the others. Each member is extracted by its occurrence into a directory of its own.
    ar t "$lib" | awk '{ n[$0]++; print n[$0] "\t" $0 }' | while IFS="$(printf '\t')" read -r count name; do
        case "$name" in *.o) ;; *) continue ;; esac
        mkdir -p "$work/m/$count"
        (cd "$work/m/$count" && ar xN "$count" "$lib" "$name")
    done
    find "$work/m" -name '*.o' > "$work/objects"
    xargs ld -r -o "$work/all.o" < "$work/objects"

    # GNU nm crashes on these objects through its LLVM plugin, which is why readelf lists them.
    readelf -Ws "$work/all.o" | awk '$5 == "GLOBAL" && $7 != "UND" { print $8 }' | grep '^rimg_' | sort -u > "$work/api.txt"
    [ -s "$work/api.txt" ] || { echo "no rimg_ symbols found" >&2; exit 1; }

    # `-R .group` is not optional: without it lld refuses the link because every Rust object
    # carries a COMDAT group named DW.ref.rust_eh_personality. .llvmbc and .llvmcmd are LLVM
    # bitcode that std's objects carry for LTO; nothing links against it after this point, and it
    # is most of the archive's size.
    objcopy --keep-global-symbols="$work/api.txt" -R .group -R .llvmbc -R .llvmcmd "$work/all.o" "$out/rust_image_ffi.o"
    # Local symbols the relocations do not need go too; the global rimg_ ones stay.
    strip --strip-unneeded "$out/rust_image_ffi.o"
    artifact=rust_image_ffi.o
    exported=$(readelf -Ws "$out/$artifact" | awk '$5 == "GLOBAL" && $7 != "UND" { print $8 }' | grep -c '^rimg_')
    expected=$(wc -l < "$work/api.txt")
    ;;
macho)
    lib="$base/librust_image_ffi.a"
    # ld64 takes the archive whole with -all_load, so the duplicate-member dance above is not
    # needed. -exported_symbols_list globs, and everything it does not name becomes private extern.
    #
    # -arch is not optional for a partial link, and the way it fails is not a message: without it
    # ld reads the archive as "building for -unknown", ignores it, and then trips over an
    # assertion of its own in the objc pass.
    case "${target:-$(uname -m)}" in
        x86_64*) arch=x86_64 ;;
        aarch64* | arm64*) arch=arm64 ;;
        *) echo "unknown macOS architecture: ${target:-$(uname -m)}" >&2; exit 1 ;;
    esac
    # The LLVM tools of this very rustc: they know Mach-O, and they read its bitcode, which
    # Xcode's own tools may be too old for. Asked from the repository, so rust-toolchain.toml
    # picks the compiler rather than whatever the caller's shell defaults to.
    tools="$(cd "$root" && rustc --print sysroot)/lib/rustlib/$(cd "$root" && rustc -vV | sed -n 's/^host: //p')/bin"
    if [ ! -x "$tools/llvm-objcopy" ]; then
        echo "llvm-objcopy not found in $tools -- rustup component add llvm-tools" >&2
        exit 1
    fi

    # The one symbol that cannot be made local. compiler_builtins is never part of LTO, and its
    # objects refer to _rust_eh_personality as an undefined external. On ELF, objcopy localizes
    # after the partial link, when those references already point at the definition. On Mach-O
    # the export list is applied inside ld -r, the definition becomes private extern, and the
    # references stay behind unbound: "Undefined symbols: _rust_eh_personality" at the caller's
    # link. Renaming it is not possible either -- llvm-objcopy leaves undefined Mach-O symbols
    # alone. So it stays global, but weak: its references resolve as in any partial link, and a
    # second Rust staticlib in the same binary brings its own personality without a duplicate
    # symbol -- the linker keeps one of them.
    "$tools/llvm-objcopy" --weaken-symbol _rust_eh_personality "$lib" "$work/weak.a"
    lib="$work/weak.a"
    printf '%s\n' '_rimg_*' '_rust_eh_personality' > "$work/api.txt"
    # -platform_version only silences ld's "no platform load command found" in every build that
    # links this object later. Old linkers do not know the flag, so a refusal falls back rather
    # than failing the release over a warning.
    sdk=$(xcrun --show-sdk-version 2> /dev/null || echo 11.0)
    ld -r -arch "$arch" -platform_version macos 11.0 "$sdk" -all_load "$lib" \
        -exported_symbols_list "$work/api.txt" -o "$work/all.o" 2> "$work/ld.err" || {
        cat "$work/ld.err" >&2
        ld -r -arch "$arch" -all_load "$lib" -exported_symbols_list "$work/api.txt" -o "$work/all.o"
    }
    # The __LLVM segment is the bitcode rustc carries for LTO -- what -R .llvmbc removes on ELF.
    # Nothing links against it from here on, it is most of the size, and it is not inert: Xcode's
    # nm reads it and fails on it, because the bitcode is LLVM 20 from rustc and the tool is LLVM
    # 15 ("Unknown attribute kind"). A consumer's toolchain would meet the same thing.
    if xcrun --find bitcode_strip > /dev/null 2>&1 &&
        xcrun bitcode_strip -r "$work/all.o" -o "$work/nobitcode.o" 2> /dev/null; then
        mv "$work/nobitcode.o" "$work/all.o"
    else
        echo "note: bitcode_strip is not available; the object keeps its __LLVM segment" >&2
    fi
    cp "$work/all.o" "$out/rust_image_ffi.o"
    # No `strip -x` here, however tempting the size is. -exported_symbols_list has already made
    # everything but the rimg_ names private extern, which is what keeps two Rust staticlibs from
    # colliding -- and private extern is *local*, so -x deletes it. Including
    # _rust_eh_personality, which the unwind tables point at: the object then links with
    # "Undefined symbols: _rust_eh_personality". ELF has objcopy --strip-unneeded, which keeps
    # what relocations need; Mach-O's strip has no such promise.
    artifact=rust_image_ffi.o
    # Every prototype in the header starts in column one and names its function before a "(".
    expected=$(grep -c '^[a-z].*[ *]rimg_[a-z_]*(' "$root/include/rust_image_ffi.h")
    if "$tools/llvm-nm" "$out/$artifact" > "$work/symbols.txt" 2> "$work/nm.err"; then
        exported=$(grep -c ' T _rimg_' "$work/symbols.txt" || true)
        # What the smoke link would find out a step later, said here with its reason.
        if grep -q ' U _rust_eh_personality$' "$work/symbols.txt"; then
            echo "the object still refers to _rust_eh_personality without binding it to its own" >&2
            echo "definition; a caller's link would fail with an undefined symbol." >&2
            exit 1
        fi
    else
        # Counting symbols is the quick check, not the proof: what proves this object is the
        # smoke link that follows it, and a symbol that did not stay global fails that link with
        # "undefined symbol". So a tool that cannot read the file says so and steps aside.
        echo "note: nm could not read the object, so the symbol count is skipped." >&2
        echo "      The C link in scripts/c-smoke.sh is what has to pass. nm said:" >&2
        sed 's/^/      /' "$work/nm.err" >&2
        exported=$expected
    fi
    ;;
coff)
    # MSVC names it rust_image_ffi.lib, MinGW librust_image_ffi.a.
    case "$target" in
        *-gnu | *-gnullvm) artifact=librust_image_ffi.a ;;
        *) artifact=rust_image_ffi.lib ;;
    esac
    cp "$base/$artifact" "$out/$artifact"
    # rustc names the libraries the object needs but not where it found them, and not all of them
    # need be Windows': a crate may ship an import library of its own inside the registry, as
    # windows-targets does. A caller with nothing but this archive cannot link without it, so
    # whatever is not on the linker's own search path travels with the artifact.
    for token in $(cat "$out/NATIVE_LIBS.txt"); do
        case "$token" in *.lib) ;; *) continue ;; esac
        [ -f "$out/$token" ] && continue
        found=$(find "${CARGO_HOME:-$HOME/.cargo}/registry/src" -name "$token" 2> /dev/null | head -n 1)
        if [ -n "$found" ]; then
            cp "$found" "$out/$token"
            echo "carried along: $token"
        fi
    done
    # No symbol table reader that is there on every Windows runner without the MSVC environment;
    # what proves this artifact is the smoke link in scripts/c-smoke.sh, which fails loudly.
    exported=0
    expected=0
    ;;
esac

if [ "$exported" -lt "$expected" ]; then
    echo "only $exported of $expected rimg_ symbols are exported" >&2
    exit 1
fi

cp "$root/include/rust_image_ffi.h" "$out/"
# Everything in the directory, whatever the platform put there.
(cd "$out" && rm -f SHA256SUMS && set -- * && sums "$@" > SHA256SUMS)
echo "$out/$artifact: $exported global symbols, $(du -h "$out/$artifact" | cut -f1)"
echo "link with: $(cat "$out/NATIVE_LIBS.txt")"

#!/bin/sh
# Links tests/c/smoke.c against what dist/<target> ships and runs it, with the C toolchains a
# caller of that target would use:
#
#   glibc Linux, macOS    the system's cc and, when present, zig cc as a second opinion
#   *-linux-musl          zig cc for musl, and Alpine's own cc in a container when Docker is there
#   *-windows-gnu         mingw-w64's gcc on Windows; zig cc elsewhere, linked and not run
#   *-windows-gnullvm     zig cc, run on Windows and only linked elsewhere
#
#   scripts/c-smoke.sh [target-triple]   uses dist/<triple or host>, building it first when it
#                                        is missing
#   SMOKE_LINK_ONLY=1 scripts/c-smoke.sh <triple>
#                                        links but does not run: a cross-built object can be
#                                        linked on this machine and not executed on it
#
# Not for *-windows-msvc: there is no compiler on PATH without the MSVC environment, and
# .github/workflows/prebuild.yml does the same link there with cl.
set -eu

target=${1:-}
root=$(cd "$(dirname "$0")" && cd .. && pwd)
dist="$root/dist/${target:-host}"
[ -f "$dist/NATIVE_LIBS.txt" ] || "$root/scripts/localize.sh" ${target:+"$target"}

artifact="$dist/rust_image_ffi.o"
[ -f "$artifact" ] || artifact="$dist/librust_image_ffi.a"

# What rustc said this object needs, rather than a list that drifts: -lgcc_s -lutil -lrt ... on
# Linux, -lSystem and a framework or two on macOS.
libs=$(cat "$dist/NATIVE_LIBS.txt")

# zig: on PATH, where gradido's shared-native keeps its own, or the one pip installs -- which is
# how the workflow gets it, without an action from a third party.
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

case "$(uname -s)" in
    MINGW* | MSYS* | CYGWIN*) on_windows=1 ;;
    *) on_windows=0 ;;
esac

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
ran=0

# run <name> <may it run here: 1|0> <compiler and its flags...>
run() {
    name=$1
    runnable=$2
    shift 2
    # The libraries come last: a static object's undefined symbols are resolved left to right.
    "$@" -std=c11 -O2 -Wall -Werror -I "$dist" "$root/tests/c/smoke.c" "$artifact" \
        -o "$work/smoke-$name$exe" $libs
    ran=1
    if [ "${SMOKE_LINK_ONLY:-0}" = 1 ] || [ "$runnable" = 0 ]; then
        echo "$name: linked (not run: built for $target)"
        return 0
    fi
    printf '%s: ' "$name"
    "$work/smoke-$name$exe"
}

exe=""
case "$target" in
*-linux-musl)
    arch=${target%%-*}
    # A static musl program runs on any Linux of its architecture, this one included.
    same=0
    [ "$(uname -s)" = Linux ] && [ "$(uname -m)" = "$arch" ] && same=1
    if [ -n "$zig" ]; then
        run zig "$same" $zig cc -target "$arch-linux-musl" -lunwind
    fi
    # The toolchain an Alpine user has. The container is this machine's architecture, so it is
    # only asked when that is the target's.
    if [ "$same" = 1 ] && [ "${SMOKE_LINK_ONLY:-0}" != 1 ] && docker info > /dev/null 2>&1; then
        printf 'alpine cc: '
        docker run --rm -v "$root:/src:ro" -e LIBS="$libs" -e DIST="/src/dist/$target" alpine:3 sh -c '
            apk add --no-cache gcc musl-dev > /dev/null &&
            cc -std=c11 -O2 -Wall -Werror -I "$DIST" /src/tests/c/smoke.c "$DIST/rust_image_ffi.o" \
                -o /tmp/smoke $LIBS && /tmp/smoke'
        ran=1
    fi
    ;;
*-windows-gnullvm)
    exe=.exe
    [ -n "$zig" ] && run zig "$on_windows" $zig cc -target "${target%%-*}-windows-gnu"
    ;;
*-windows-gnu)
    exe=.exe
    if [ "$on_windows" = 1 ] && command -v gcc > /dev/null 2>&1; then
        run gcc 1 gcc
    elif [ -n "$zig" ]; then
        # zig has no libgcc; its libunwind stands in for the unwinder gcc would bring.
        run zig "$on_windows" $zig cc -target "${target%%-*}-windows-gnu" -lunwind
    fi
    ;;
*-windows-msvc)
    echo "c-smoke.sh does not link for MSVC; see .github/workflows/prebuild.yml" >&2
    exit 1
    ;;
x86_64-apple-darwin)
    # An object built for another architecture still links here; it is the link that is being proven.
    run cc 1 "${CC:-cc}" -arch x86_64
    ;;
aarch64-apple-darwin)
    run cc 1 "${CC:-cc}" -arch arm64
    ;;
*)
    run cc 1 "${CC:-cc}"
    # zig is the second opinion on the same host, not a cross toolchain.
    [ -n "$zig" ] && run zig 1 $zig cc -lunwind
    ;;
esac

if [ "$ran" = 0 ]; then
    echo "no toolchain found that links for ${target:-this host}; nothing was proven" >&2
    exit 1
fi

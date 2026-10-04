#!/bin/sh
# Checks Cargo.lock against the RustSec advisory database: fails when a crate this module is
# built from has a known vulnerability. Warnings -- an unmaintained or yanked crate -- are printed
# and do not fail it.
#
#   scripts/audit.sh
#
# Installs cargo-audit when it is missing, with the stable toolchain rather than the one pinned in
# rust-toolchain.toml: the pin is for what ships, and the tool that reads a lockfile need not be
# built with it.
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)

if ! command -v cargo-audit > /dev/null 2>&1; then
    cargo +stable install cargo-audit --locked
fi
# From the repository, so that Cargo.lock is this one. fuzz/ has a lockfile of its own for a tool
# that never ships; it is not audited.
cd "$root" && cargo audit

#!/bin/bash
#:
#: name = "build-and-test"
#: variety = "basic"
#: target = "helios-latest"
#: rust_toolchain = "stable"
#: output_rules = []
#:

set -o errexit
set -o pipefail
set -o xtrace

cargo --version
rustc --version

banner fmt
cargo fmt --check

banner build
ptime -m cargo build --all-targets

banner clippy
ptime -m cargo clippy --all-targets -- -D warnings

banner test
cargo install cargo-nextest --locked
# Run tests under libumem audit mode so use-after-free, double-free, and
# buffer-overrun bugs in libtopo (or our FFI usage of it) surface as
# deterministic SIGSEGVs instead of being silently tolerated by the
# default umem allocator.
pfexec ptime -m env \
	LD_PRELOAD=libumem.so.1 \
	UMEM_DEBUG=default,audit=16,contents \
	UMEM_LOGGING=transaction \
	cargo nextest run

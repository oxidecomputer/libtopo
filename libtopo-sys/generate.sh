#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")"

bindgen wrapper.h \
    --allowlist-function 'topo_.*' \
    --allowlist-type 'topo_.*|tnode_.*' \
    --allowlist-var 'TOPO_.*|FM_.*' \
    --raw-line '#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]' \
    > src/lib.rs

# Normalize formatting so `cargo fmt --check` is clean in CI; bindgen's
# internal rustfmt invocation can disagree with the workspace style on
# continuation indents.
cargo fmt -p libtopo-sys


#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")"

bindgen wrapper.h \
    --allowlist-function 'topo_.*' \
    --allowlist-type 'topo_.*|tnode_.*' \
    --allowlist-var 'TOPO_.*|FM_.*' \
    --allowlist-file '.*/fm/topo_hc\.h' \
    --raw-line '#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]' \
    > src/lib.rs

# Normalize formatting so `cargo fmt --check` is clean in CI; bindgen's
# internal rustfmt invocation can disagree with the workspace style on
# continuation indents.
cargo fmt -p libtopo-sys

# Record which string macros <fm/topo_hc.h> defines, so libtopo's tests can
# check that libtopo::hc re-exports every one of them.
hc_header=$(echo '#include <fm/topo_hc.h>' | "${CC:-gcc}" -E -M -x c - \
    | tr -s ' \\' '\n' | grep '/fm/topo_hc\.h$')
{
    echo "# String macros defined by <fm/topo_hc.h>."
    echo "# Written by libtopo-sys/generate.sh; do not edit by hand."
    grep -E '^#define[[:space:]]+[A-Za-z0-9_]+[[:space:]]+"' "$hc_header" \
        | awk '{ print $2 }'
} > ../libtopo/src/topo_hc_names.txt


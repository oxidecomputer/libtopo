#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")"

bindgen wrapper.h \
    --raw-line '#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]' \
    > src/lib.rs


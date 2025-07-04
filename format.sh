#!/usr/bin/env bash
set -euo pipefail

# Format code with import nesting and sorting
# Using unstable cargo fmt options (allowed on command line even if not in config)
cargo fmt -- \
    --config imports_granularity=Crate \
    --config group_imports=StdExternalCrate
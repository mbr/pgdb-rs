#!/bin/sh

#: Formats the source.
#: Uses --config to override rustfmt settings without a nightly toolchain.
#: As a little hack, supports `--check`.

cd "$(dirname "$0")"

cargo fmt -- --config group_imports=StdExternalCrate --config imports_granularity=Crate "$@"
nixfmt "$@" flake.nix

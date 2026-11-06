#!/usr/bin/env bash
# Builds mulac-outbox across its feature combinations and proves that Diesel is
# absent from the dependency tree whenever the `diesel` feature is off. Each
# check selects only the package, so another crate's features cannot leak in.
set -euo pipefail

manifest="$(dirname "$0")/../libs/Cargo.toml"

# Combinations that include Diesel: default, and everything.
echo "== default features"
cargo check --manifest-path "$manifest" -p mulac-outbox --all-targets
echo "== all features"
cargo check --manifest-path "$manifest" -p mulac-outbox --all-targets --all-features

# Positive control: the absence checks below only mean something if this finds
# diesel when the feature is on, rather than failing for an unrelated reason.
cargo tree --manifest-path "$manifest" -p mulac-outbox --no-default-features --features diesel -e normal -i diesel >/dev/null

# Combinations without Diesel: no features, and each API/HTTP adapter shape.
without_diesel=(
  ""
  "api"
  "api-tokio"
  "api-testing"
  "axum"
  "axum-openapi"
  "poem"
  "amqp"
  "axum,poem,api-testing"
  "axum-openapi,poem,amqp,api-testing"
)
for features in "${without_diesel[@]}"; do
  flags=(--no-default-features)
  [ -n "$features" ] && flags+=(--features "$features")
  echo "== without diesel: ${features:-no features}"
  cargo check --manifest-path "$manifest" -p mulac-outbox --all-targets "${flags[@]}"
  if cargo tree --manifest-path "$manifest" -p mulac-outbox "${flags[@]}" -e normal -i diesel >/dev/null 2>&1; then
    echo "diesel is in the dependency tree of: ${features:-no features}" >&2
    exit 1
  fi
done
echo "all feature combinations build, and diesel is absent whenever it is disabled"

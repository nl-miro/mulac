#!/usr/bin/env bash
# Runs the outbox payload corpus in package-isolated invocations: a baseline and
# every downstream combination of the serde_json features that could change
# JSON handling. The resolved serde_json features of each run are printed so
# the record shows that the baseline was not augmented by feature unification.
#
# Needs OUTBOX_TEST_DATABASE_URL (a disposable PostgreSQL database) for the
# equivalence cases; the validation cases run without it.
set -euo pipefail

manifest="$(dirname "$0")/../test_apps/outbox_serde_matrix/Cargo.toml"
augmenting='serde_json feature "(arbitrary_precision|preserve_order|raw_value)"'
combinations=("" "arbitrary_precision" "preserve_order" "raw_value" "arbitrary_precision,preserve_order,raw_value")

for combo in "${combinations[@]}"; do
  label="${combo:-baseline}"
  flags=()
  [ -n "$combo" ] && flags=(--features "$combo")
  echo "=== serde_json features: $label"
  resolved="$(cargo tree --manifest-path "$manifest" "${flags[@]}" -e features,dev -i serde_json 2>&1)"
  echo "$resolved" | grep 'serde_json feature' | sort -u
  if [ -z "$combo" ] && echo "$resolved" | grep -Eq "$augmenting"; then
    echo "baseline resolved an augmented serde_json: feature unification leaked in" >&2
    exit 1
  fi
  cargo test --manifest-path "$manifest" "${flags[@]}" -- --include-ignored --nocapture
done

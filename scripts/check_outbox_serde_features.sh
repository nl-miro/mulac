#!/usr/bin/env bash
# Runs the outbox payload corpus in package-isolated invocations: a baseline and
# every one of the 2^3 downstream combinations of the serde_json features that
# could change JSON handling (eight runs in all). The resolved serde_json
# features of each run are printed and must be exactly the requested ones, so the
# record shows that no run was augmented by feature unification.
#
# Needs OUTBOX_TEST_DATABASE_URL (a disposable PostgreSQL database) for the
# equivalence cases; the validation cases run without it.
set -euo pipefail

manifest="$(dirname "$0")/../test_apps/outbox_serde_matrix/Cargo.toml"
features=(arbitrary_precision preserve_order raw_value)
combinations=("")
for mask in 1 2 3 4 5 6 7; do
  combo=""
  for i in 0 1 2; do
    if (( mask & (1 << i) )); then combo="${combo:+$combo,}${features[i]}"; fi
  done
  combinations+=("$combo")
done

for combo in "${combinations[@]}"; do
  label="${combo:-baseline}"
  flags=()
  [ -n "$combo" ] && flags=(--features "$combo")
  echo "=== serde_json features: $label"
  resolved="$(cargo tree --manifest-path "$manifest" "${flags[@]}" -e features,dev -i serde_json 2>&1)"
  echo "$resolved" | grep 'serde_json feature' | sort -u
  # Each run must resolve exactly the requested features: a baseline or partial
  # combination silently augmented by feature unification would prove nothing.
  for feature in "${features[@]}"; do
    wanted=0
    case ",$combo," in *",$feature,"*) wanted=1 ;; esac
    present=0
    echo "$resolved" | grep -q "serde_json feature \"$feature\"" && present=1
    if [ "$wanted" != "$present" ]; then
      echo "$label: serde_json feature $feature requested=$wanted resolved=$present: feature unification leaked in" >&2
      exit 1
    fi
  done
  cargo test --manifest-path "$manifest" "${flags[@]}" -- --include-ignored --nocapture
done

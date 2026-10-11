#!/usr/bin/env bash
set -euo pipefail

# Compile and run the public-library integration tests with the module adapter
# absent, then with every optional library surface enabled.
cargo test --package tinymcp --no-default-features --test library_feature_tests
cargo test --package tinymcp --no-default-features --features tools,ui,server-http --test library_feature_tests

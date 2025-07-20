# Format all Rust code in the repository
fmt:
    cargo fmt --all

# Check if all Rust code is formatted correctly (useful for CI)
fmt-check:
    cargo fmt --all -- --check
# Architecture

This repo uses a pipeline architecture with strict unidirectional data flow.
Each module has a defined seam (trait) with no cross-module internal dependencies.

# Boundaries

Never: modify generated files in src/generated/
Always: run cargo fmt before pushing
Ask first: adding new workspace dependencies

# Stack

- Rust 1.78 stable
- tokio 1.x async runtime
- clap 4 for CLI

# Commands

```bash
cargo build --release
cargo test --workspace
```

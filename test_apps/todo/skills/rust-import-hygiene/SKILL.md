# Rust import hygiene

Use this skill when a Rust file or group of Rust files in this repository needs import cleanup.

## Goal

Make imports consistent and easy to scan.

## Rules

- keep imports at the top of their module
- do not leave empty lines between import statements
- use grouped imports for multiple bindings from the same crate when practical
- remove imports that are no longer used
- do not leave imports below code in the same module

## Workflow

1. Inspect the target file or files.
2. Find import statements that are:
   - separated by empty lines
   - duplicated from the same crate
   - placed below code in the module
   - no longer used
3. Rewrite imports to follow the repository rules.
4. Run formatting.
5. Run a compile check.
6. Report any remaining warnings that are unrelated to import hygiene.

## Preferred patterns

```rust
use crate::assembly::io::{ApiError, AppError, TodoEntry};
use poem_openapi::{Object, OpenApi, payload::Json};
use serde::{Deserialize, Serialize};
```

## Avoid

```rust
use crate::assembly::io::ApiError;

use crate::assembly::io::AppError;
use crate::assembly::io::TodoEntry;
```

```rust
fn something() {
    ...
}

use uuid::Uuid;
```

## Validation

After changes, prefer:

```bash
cargo fmt --all
cargo check
```

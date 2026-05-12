# Developer Guidelines

These guidelines describe the preferred coding and organization style for this repository.

## Import rules

### Group imports from the same top module

Combine bindings from the same top-level module into a single `use` statement:

```rust
use crate::application::io::{InboxError, InboxMessageEnvelope, InboxProcessPort};
```

Prefer this over separate lines:

```rust
use crate::application::io::InboxError;
use crate::application::io::InboxMessageEnvelope;
use crate::application::io::InboxProcessPort;
```

### Force multiline formatting for large import groups

When a `use` group has more than 3 bindings, add a trailing `//` after the last element to prevent rustfmt from collapsing it to a single line:

```rust
pub use crate::application::io::{
    InboxError,
    InboxMessageEnvelope,
    InboxMessageMetadata,
    InboxProcessPort, //
};
```

Groups with 3 or fewer bindings may stay on one line.

## Rust style

### Prefer `?` over manual error matches

Use `map_err` with `?` when converting error types:

```rust
let pool = build_pool(&database_url).map_err(|e| e.to_string())?;
```

Prefer this over:

```rust
let pool = match build_pool(&database_url) {
    Ok(pool) => pool,
    Err(e) => return Err(e.to_string()),
};
```

If the surrounding function returns `anyhow::Result<_>`, add context at the boundary:

```rust
let repository = repository(config.database_url)
    .map_err(|e| anyhow::anyhow!("failed to initialize repository: {e}"))?;
```

### Prefer `let else` for required `Option` values

Use `let else` when a missing value should skip or return early:

```rust
let Some(reservation_id) = message.reservation_id() else {
    errors.push(InboxError::MissingReservation { id });
    continue;
};
```

Keep `match` when the error value is needed and the match remains clearer:

```rust
let cmd: Command = match message.try_into() {
    Ok(cmd) => cmd,
    Err(e) => {
        errors.push(e);
        continue;
    }
};
```

### Avoid unnecessary clones

Only clone when a value is reused or shared ownership is required:

```rust
let repository = InboxRecorderRepository::new(store);
```

Prefer this over:

```rust
let repository = InboxRecorderRepository::new(store.clone());
```

when `store` is not used afterward.

### Use `Arc<dyn Trait>` for cloneable shared dependencies

Use `Arc<dyn Trait>` when repositories or services need cheap cloning and shared ownership across workers:

```rust
pub struct InboxRecorderRepository {
    store: Arc<dyn InboxStorePort>,
}
```

Use generics only when you explicitly want the containing type to become generic:

```rust
pub struct InboxRecorderRepository<S> {
    store: S,
}
```

## Module organization

### Keep dependency direction clear

Preferred direction:

```text
domain <- application <- features <- infra adapters
```

Avoid application modules depending on feature-specific modules. If an application trait needs a type, that type likely belongs in `application` rather than `features`.

### Separate repositories by use case

Prefer feature-specific repositories over one generic repository:

```text
record_messages/InboxRecorderRepository
inbox_consumer/InboxConsumerRepository
```

Each repository should depend only on the ports needed by that feature.

### Separate storage adapters by use case

Prefer concrete storage adapters that match repository responsibilities:

```text
InboxStoreStorage      // implements InboxStorePort
InboxConsumerStorage   // implements InboxReservePort + InboxProcessPort
```

Avoid one large storage type implementing unrelated ports unless the use cases are intentionally coupled.

### Keep infrastructure in infra modules

Concrete adapters belong in `infra_*` modules:

```text
infra_diesel/
infra_amqp/
```

Feature modules should express use cases and ports, not database or broker implementation details.

### Keep public API behind `io.rs`

External callers should import from the root facade:

```rust
use inbox::io::{InboxRecorder, InboxRecorderRepository};
```

Inside the crate, prefer direct internal module paths instead of importing from `crate::io`.

## Inbox-specific rules

### Reservation and processing

- Reserving a message owns it by assigning a `reservation_id`.
- Processing must use the reservation ID to prevent double completion/failure.
- A missing `reservation_id` is an error, not a panic.
- Failed messages should be released and scheduled according to retry policy.
- Completed messages should clear reservation ownership unless historical reservation data is intentionally retained.

### Attempt counting

Attempts are incremented when a message is reserved for processing. Failure handling should use the already-incremented attempt count.

### Avoid leaking implementation details

Internal query criteria, database row models, and transport-specific message types should not be part of the public API unless callers need them directly.

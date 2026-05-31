# Commit `1a48e85` — small, dependency-ordered checklist (outside `test_apps/*`)

Use this order to split the commit into small, reviewable changes.

## 1) Add derive crate foundation (independent)
- [ ] Add `kernel_derive` crate (`Cargo.toml`, `src/lib.rs`).
- [ ] Add `kernel_derive` dependency to `kernel/Cargo.toml`.
- [ ] Re-export derives from `kernel/src/lib.rs`:
  - `ApplicationCommand`
  - `ApplicationEvent`

## 2) Refactor handler/subscriber registration types (depends on 1 only if merged together)
- [ ] Add `CommandHandlers` and `EventSubscribers` wrappers in `kernel/src/lib.rs`.
- [ ] Add `KernelBuilder::command_handlers(...)` and `KernelBuilder::event_subscribers(...)`.
- [ ] Update existing builder methods to append into wrapper `.items`.
- [ ] Update `CommandHandlerRegistry::from_handlers(...)` to consume `CommandHandlers`.

## 3) Persistent inbox wiring (depends on 2 only for conflict management)
- [ ] Add `inbox_recorder` to `PersistentKernelState`.
- [ ] Wire recorder during `start_persistent(...)`.
- [ ] Replace `NoopInboxStore` in persistent path with real durable store (or explicitly gate until available).

## 4) No-op inbox visibility cleanup (depends on 3)
- [ ] Keep `NoopInboxStore` private unless external access is required.
- [ ] If public exposure is required, document its intended non-persistent/test-only usage.

## 5) Commanding error surface update (independent)
- [ ] Add `CommandDispatchError::Domain(String)` in `libs/commanding/src/assembly/application.rs`.
- [ ] Ensure all call sites map domain errors intentionally (not via generic fallthrough).

## 6) Formatting-only outbox delta (independent; last)
- [ ] Keep `libs/outbox/src/outbox_consumer.rs` signature reflow as a standalone formatting commit.

## 7) Repo housekeeping (independent; optional)
- [ ] Separate `Makefile` `wip` target into its own commit.
- [ ] Separate `TODO.md` edits into docs-only commit.

## Suggested small-PR sequence

1. **PR-A (derive crate only)**  
   Includes: section 1.
2. **PR-B (builder registry refactor)**  
   Includes: section 2.
3. **PR-C (persistent inbox correctness)**  
   Includes: section 3 + section 4.
4. **PR-D (commanding domain error variant)**  
   Includes: section 5.
5. **PR-E (formatting-only outbox change)**  
   Includes: section 6.
6. **PR-F (repo housekeeping)**  
   Includes: section 7.

## Dependency gates (do not skip)

- [ ] **Gate 1:** PR-A merged before any derive usage in downstream crates.
- [ ] **Gate 2:** PR-B merged before touching registry wiring call sites.
- [ ] **Gate 3:** PR-C must include a regression test proving persistent inbox writes are durable.
- [ ] **Gate 4:** PR-D must include at least one call-site mapping to `CommandDispatchError::Domain`.

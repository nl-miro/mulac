# Review: `test_apps/twitter`

Reviewed against `docs/codestyle.md` and the current crate structure.

## Summary

The app is mostly clean, but there are a few important issues: one response contract looks wrong, shutdown is not fully graceful, tests are missing for key DB-backed behavior, and the crate still exposes internals more broadly than its facade suggests.

## Findings

1. **High — unfollow response misrepresents resulting state**
   - `test_apps/twitter/src/user_unfollow.rs:108-143`
   - `test_apps/twitter/src/inbox.rs:177-211`
   - `unfollow` returns a synthetic `FollowDto` / `InboundEntity::Follow` after the follow row has been deleted. That is inconsistent with the delete/unlike style and can mislead callers about current state.
   - **Suggested fix:** return `204 No Content`, or introduce a dedicated unfollow response contract and make inbox materialization match it.

2. **High — worker shutdown is not actually awaited**
   - `test_apps/twitter/src/assembly/application.rs:254-284`
   - `test_apps/twitter/src/assembly/bin/twitterapp.rs:63-98`
   - `MulacHandle::wait()` cancels the token but does not join worker tasks. On shutdown, in-flight work can be dropped instead of drained cleanly.
   - **Suggested fix:** store worker `JoinHandle`s and await them, or make `wait()` block until workers have exited.

3. **Medium — test coverage misses core DB-backed behavior**
   - `test_apps/twitter/Cargo.toml`
   - `test_apps/twitter/src/*`
   - There are no crate-local tests proving round trips, idempotency, and shutdown behavior for the app itself.
   - **Suggested fix:** add integration coverage for post/follow/unfollow/like/delete flows, idempotency cases, and shutdown/worker behavior.

4. **Low — facade boundary is still too wide**
   - `test_apps/twitter/src/lib.rs:1-126`
   - The crate still exposes `pub mod schema` and root-public types instead of forcing consumers through the `io` facade.
   - **Suggested fix:** privatize internals where possible and re-export only through `io`.

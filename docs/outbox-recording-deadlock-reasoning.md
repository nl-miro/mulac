# Why batches that hold no other locks cannot deadlock

Status: written by the implementer; **awaiting the repository owner's review** on
the delivering pull request. The plan requires the reasoning to be written down
and reviewed, and this file is the first half of that.

## The guarantee

At `READ COMMITTED`, outbox batches that hold no other locks do not deadlock with
each other, whatever order their events are supplied in. It assumes the
documented schema (`docs/outbox_entries.sql`) without application-added locking
triggers, constraints or concurrent schema changes.

It does **not** extend to a transaction that already holds locks, including locks
taken by an earlier recording call in the same transaction or by the
application's own writes. See [what is not covered](#what-is-not-covered).

## What a batch locks

A recording batch does two things to the table:

1. **Inserts new rows**, with `INSERT … ON CONFLICT (id) DO NOTHING`. Each new
   row stays uncommitted, and so blocks anyone else inserting the same event ID,
   until the batch's transaction (or the caller's) ends. Inserting an ID that is
   being inserted by an uncommitted transaction **waits** for that transaction.
2. **Verifies replays**, with a plain `SELECT` of the rows that were skipped.
   This takes no row lock: it neither waits for nor blocks any other session.
   Nothing in the engine uses `FOR SHARE`, `FOR UPDATE` or an explicit lock.

So the only thing a batch can ever wait for is another transaction's
uncommitted insert of the same event ID.

## The argument

Every batch inserts its rows in ascending event-ID order, and that order is
global:

- the batch is sorted once by event ID, whatever order it was supplied in;
- it is split into consecutive chunks of that sorted list, and the chunks are
  executed in order, so every ID in an earlier statement is smaller than every ID
  in a later one;
- inside one multi-row `INSERT`, PostgreSQL processes the `VALUES` rows in the
  order listed.

Take any batch `A` that is waiting. It is blocked at some ID `x`, which means a
different batch `B` has already inserted `x` and not yet committed. Because `B`
inserts in ascending order, `B` has finished everything up to `x`, so wherever `B`
is now, it is at an ID **strictly greater than `x`**: it is either running, or
waiting at some `y > x`. (It cannot be at `x`, since it already inserted it, and
IDs are unique within a batch.)

So every wait edge goes from a batch at position `x` to a batch at a strictly
larger position. A cycle of waits would have to return to its starting position
after only increasing it, which is impossible. Therefore the wait-for graph
among such batches has no cycle, and PostgreSQL's deadlock detector is never
needed.

Statement and chunk boundaries do not weaken this, because the order is one
global order, not an order per statement. That is exactly the property a
per-statement sort would lose: two batches that overlap across a boundary could
each hold half of the shared range, and wait for the other half.

## Consumers, reservation and sweep

Reservation, finalization and the stale sweep touch **committed** rows. Recording
never waits on them, and they never wait on a batch's uncommitted new rows:

- reservation uses `FOR UPDATE SKIP LOCKED` over rows it can see, and an
  uncommitted insert is invisible to it;
- the sweep locks only visible `Reserved` rows;
- replay verification takes no row lock, so it cannot hold up a consumer, and a
  consumer's row lock cannot hold up a plain read.

A batch can wait on an in-progress **update** of an already-committed row only
through `ON CONFLICT DO NOTHING` on that row's ID, which waits for the updater to
finish. The updater (a consumer finalizing a row) holds nothing a recording batch
holds, so no cycle forms.

## What is not covered

- **A transaction that already holds locks.** Inside a caller's transaction, rows
  inserted by an earlier recording call, or by the application's own statements,
  are held while a later call waits. Two such transactions can then deadlock, and
  PostgreSQL aborts one of them. The recording call that is the victim reports a
  rolled-back database failure whose source is the original database error, and
  the caller abandons the whole transaction and retries all of it.
- **Isolation above `READ COMMITTED`.** A concurrent write of the same event ID
  raises a serialization failure instead of being skipped. It is reported the
  same way and is retried the same way.
- **Application-added triggers, constraints or schema changes.** These can take
  locks this argument does not know about.
- **Database-reported deadlocks cannot be told apart** from other database errors
  by the error alone (the driver exposes no SQLSTATE), which is why the retry
  rule covers every database failure in a caller's transaction.

## Evidence

The argument is checked by tests that run against real PostgreSQL in the
dedicated CI job (`libs/outbox/tests/recording_concurrency.rs`):

- `unsorted_control_deadlocks_where_sorted_recording_does_not` builds the same
  shape of contention with raw inserts in arrival order and shows PostgreSQL
  aborting one session with "deadlock detected". The sorted engine, given the same
  shape in `sorted_recording_does_not_deadlock_in_the_same_shape`, completes.
- `overlapping_batches_in_opposite_orders_across_chunk_boundaries_do_not_deadlock`
  holds one row past the first insertion chunk, starts one batch in ascending and
  one in descending order over more than one chunk of overlapping IDs, observes
  both blocked in the database, then releases them and requires both to succeed.
  The tests observe the blocked backends instead of relying on timing.
- Both were checked by mutation while writing them: with the global sort removed,
  they fail with "deadlock detected"; with it restored they pass.
- `deadlock_through_recording_calls_on_both_sides_has_one_victim_and_a_retry_succeeds`
  shows the uncovered case behaving as documented.

//! Recording outbox events together with business writes, with a bounded retry
//! loop. This is the handler example the outbox documentation links to; the
//! `handler_example` test includes this very file and runs it against
//! PostgreSQL.
//!
//! Rules it demonstrates:
//!
//! * **Event IDs and replay-compared content are chosen once**, in a [`Plan`],
//!   before the first attempt. A retry never builds new events. The
//!   application persists the plan and the [`History`] (for example in its own
//!   table, keyed by the business operation) so both survive handler
//!   reinvocation, delayed redrives and process restarts.
//! * **One application-owned error type** keeps recording failures, business or
//!   database failures, and the caller's own commit failure apart. Convert it
//!   explicitly into the handler or subscriber error only after it has been
//!   classified; never with a blanket `From`.
//! * **The business write must be safe to repeat.** A `COMMIT` can reach the
//!   database and its acknowledgment never reach the application. The handler
//!   cannot tell that from a commit that failed, so it runs the whole
//!   transaction again, business callback included. Key the business write by
//!   the plan's `operation_id` (for example `INSERT … ON CONFLICT DO NOTHING`),
//!   or look the operation up and reconcile before retrying. A callback that
//!   simply inserts a row inserts it twice.
//! * **The callback outcome is recorded.** `transaction` can return a rollback
//!   or commit error in place of the callback's error, so the loop notes what
//!   the callback itself returned and trusts that.
//! * **An uncertain outcome is never resolved by minting new IDs.** The loop
//!   retries the whole transaction with the same plan; if a later attempt
//!   reports a conflict, or retries run out, it stops and escalates, and the
//!   history stays unresolved.
//! * **A later success does not resolve an earlier uncertain attempt.** It
//!   satisfies the recording obligation ([`History::recorded`]) but cannot show
//!   whether the earlier attempt committed or published, so
//!   [`History::unresolved`] stays set until the application has reconciled.
//! * **Retention is a precondition, stated by the plan.** A retry with the same
//!   IDs adds nothing only while the earlier rows still exist. [`Plan`] carries
//!   `retention_assured`; when it is false the loop never retries an uncertain
//!   outcome and escalates for reconciliation instead.
use diesel::prelude::*;
use mulac_outbox::prelude::*;
use std::num::NonZeroU32;
use uuid::Uuid;

/// Everything a retry must reuse unchanged. Create it once per logical
/// operation, before the first attempt, and persist it.
pub struct Plan {
    /// Identifies the business operation. The business write is keyed by it, so
    /// a repeated attempt finds its own earlier write instead of adding another.
    pub operation_id: Uuid,
    pub events: Vec<NewOutboxEnvelope>,
    /// Set only when nothing deletes outbox rows before the supported retry and
    /// reconciliation horizon ends (no cleanup job, manual purge or retention
    /// policy shorter than it). Same-ID replay prevents a second insert, and
    /// so a second publication, only while the original row exists. When this
    /// is false an uncertain outcome is escalated for reconciliation, never
    /// retried.
    pub retention_assured: bool,
}

/// What earlier attempts established. Persist it with the [`Plan`]: while
/// `unresolved` is set, a later "no rows added" result cannot prove that an
/// earlier attempt added none.
#[derive(Default, Debug)]
pub struct History {
    pub attempts: Vec<Attempt>,
    /// An attempt had an uncertain outcome. Only the application clears it,
    /// after reconciling; a later successful attempt never does.
    pub unresolved: bool,
    /// A later attempt succeeded, so the recording obligation is satisfied
    /// (while the rows stay retained). Independent of `unresolved`.
    pub recorded: bool,
    /// Event IDs are logged whenever an attempt's outcome is uncertain.
    pub logged: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Attempt {
    Committed,
    /// This attempt is known to have committed nothing.
    NotCommitted,
    /// Rows and business writes may or may not exist.
    Uncertain,
}

/// The application's own error type. Each variant keeps its source.
#[derive(Debug)]
pub enum HandlerError {
    /// A recording call failed; see the Failures table for each case.
    Recording(RecordingError),
    /// The callback's own statements failed, or the transaction could not begin.
    Business(diesel::result::Error),
    /// The caller's own `COMMIT` failed. This is never a recording error.
    Commit(diesel::result::Error),
    /// Stop and escalate: keep the plan and history. The outcome is whatever
    /// `History::unresolved` says, and the last failure is attached.
    Escalate {
        event_ids: Vec<Uuid>,
        reason: &'static str,
        last: Box<HandlerError>,
    },
    /// The outcome is unresolved and the plan does not assure retention, so a
    /// same-ID retry could insert and publish again. Reconcile first. `last` is
    /// the failure that made the attempt uncertain, if this call had one.
    RetentionNotAssured {
        event_ids: Vec<Uuid>,
        last: Option<Box<HandlerError>>,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub enum Policy {
    /// Retrying cannot help.
    Stop,
    /// This attempt committed nothing; retry the whole transaction.
    Retry,
    /// The outcome is unknown; retry the whole transaction, keep the history.
    RetryUncertain,
}

/// Every recording case is listed, with no wildcard, so adding one breaks the build.
pub fn policy(error: &HandlerError) -> Policy {
    match error {
        HandlerError::Recording(recording) => match recording {
            RecordingError::DuplicateEventId(_)
            | RecordingError::MismatchedEventId { .. }
            | RecordingError::InvalidPayload(_)
            | RecordingError::InvalidMetadata { .. }
            | RecordingError::ReplayConflict { .. }
            | RecordingError::NoOpenTransaction => Policy::Stop,
            // Retrying with the same IDs can publish them again; never new IDs.
            RecordingError::MissingEvents(_)
            | RecordingError::NoConnection(_)
            | RecordingError::DatabaseRolledBack(_) => Policy::Retry,
            RecordingError::DatabaseUncertain(_) => Policy::RetryUncertain,
        },
        HandlerError::Business(_) => Policy::Retry,
        HandlerError::Commit(_) => Policy::RetryUncertain,
        HandlerError::Escalate { .. } | HandlerError::RetentionNotAssured { .. } => Policy::Stop,
    }
}

/// One whole transaction: the business write and the recording commit or roll
/// back together.
fn attempt(
    pool: &DbPool,
    plan: &Plan,
    business: &dyn Fn(&mut PgConnection) -> QueryResult<()>,
) -> Result<(), HandlerError> {
    let mut conn = pool.get().map_err(|error| {
        HandlerError::Business(diesel::result::Error::QueryBuilderError(Box::new(error)))
    })?;
    // What the callback itself returned, before any rollback or commit error.
    let mut callback: Option<Result<(), HandlerError>> = None;
    let outcome = conn.transaction::<(), diesel::result::Error, _>(|conn| {
        let step = business(conn)
            .map_err(HandlerError::Business)
            .and_then(|()| {
                record_batch_on_connection(conn, &plan.events).map_err(HandlerError::Recording)
            });
        let succeeded = step.is_ok();
        callback = Some(step);
        if succeeded {
            Ok(())
        } else {
            Err(diesel::result::Error::RollbackTransaction)
        }
    });
    match callback {
        // The callback's failure always wins; a failed rollback never replaces it.
        Some(Err(error)) => Err(error),
        // The callback succeeded, so any error here came from COMMIT.
        Some(Ok(())) => outcome.map_err(HandlerError::Commit),
        // The transaction never started, so the callback never ran.
        None => Err(HandlerError::Business(
            outcome.expect_err("a transaction that ran no callback must have failed"),
        )),
    }
}

fn unretained(plan: &Plan, last: Option<HandlerError>) -> HandlerError {
    HandlerError::RetentionNotAssured {
        event_ids: plan.events.iter().map(|e| e.metadata.event_id).collect(),
        last: last.map(Box::new),
    }
}

fn escalate(plan: &Plan, reason: &'static str, last: HandlerError) -> HandlerError {
    HandlerError::Escalate {
        event_ids: plan.events.iter().map(|e| e.metadata.event_id).collect(),
        reason,
        last: Box::new(last),
    }
}

/// Runs the plan with at most `max_attempts` whole-transaction attempts.
///
/// # The business callback must be safe to repeat
///
/// An uncertain outcome (a failed `COMMIT`, or a recording that could not
/// confirm its rollback) is retried by running the whole transaction again,
/// including `business`. If the earlier attempt had in fact committed, the
/// outbox rows are found again and add nothing, but `business` runs a second
/// time. It must therefore either be keyed by [`Plan::operation_id`] so that
/// repeating it changes nothing, or check for its own earlier effect first.
/// This function cannot make an arbitrary callback safe.
///
/// # What to keep
///
/// Persist the [`Plan`] (event IDs and replay-compared content, and the
/// operation ID) before the first attempt, and the [`History`] after every
/// attempt, so a restarted process or a delayed redrive reuses both. While
/// `History::unresolved` is set the operation may already have committed:
/// never replace the plan's IDs, and treat a later "fix your input" failure as
/// an escalation, not as proof that nothing was stored. A successful retry sets
/// `History::recorded` but leaves `unresolved` set: it cannot show what the
/// earlier attempt did, and only the application clears it, after reconciling.
///
/// # Retention is a precondition
///
/// Retrying an uncertain outcome with the same IDs is duplicate-safe only while
/// the earlier rows are retained. [`Plan::retention_assured`] states that. If it
/// is false, this function does not run another attempt while the history is
/// unresolved, and does not retry an uncertain failure; it returns
/// [`HandlerError::RetentionNotAssured`] so the application reconciles first.
///
/// `max_attempts` cannot be zero, so there is always a last failure to report.
pub fn handle(
    pool: &DbPool,
    plan: &Plan,
    history: &mut History,
    max_attempts: NonZeroU32,
    business: &dyn Fn(&mut PgConnection) -> QueryResult<()>,
) -> Result<(), HandlerError> {
    let mut last = None;
    for _ in 0..max_attempts.get() {
        if history.unresolved && !plan.retention_assured {
            return Err(unretained(plan, last));
        }
        let error = match attempt(pool, plan, business) {
            Ok(()) => {
                history.attempts.push(Attempt::Committed);
                // The recording obligation is met. An earlier uncertain attempt
                // stays unresolved: this success cannot show what it did.
                history.recorded = true;
                return Ok(());
            }
            Err(error) => error,
        };
        match policy(&error) {
            Policy::Stop if history.unresolved => {
                // "Fix input" no longer applies: an earlier attempt may have
                // committed. Keep the IDs and content, and escalate.
                return Err(escalate(plan, "failed after an uncertain attempt", error));
            }
            Policy::Stop => {
                history.attempts.push(Attempt::NotCommitted);
                return Err(error);
            }
            Policy::Retry => history.attempts.push(Attempt::NotCommitted),
            Policy::RetryUncertain => {
                history.unresolved = true;
                history.attempts.push(Attempt::Uncertain);
                let ids: Vec<_> = plan.events.iter().map(|e| e.metadata.event_id).collect();
                let line = format!("outbox outcome uncertain for events {ids:?}");
                eprintln!("{line}");
                history.logged.push(line);
                if !plan.retention_assured {
                    return Err(unretained(plan, Some(error)));
                }
            }
        }
        last = Some(error);
    }
    Err(escalate(
        plan,
        "retries exhausted",
        last.expect("the loop ran at least once because max_attempts is non-zero"),
    ))
}

/// Explicit conversion into a subscriber's error, after classification.
pub fn for_subscriber(error: &HandlerError) -> String {
    match error {
        HandlerError::Recording(e) => format!("outbox recording failed: {e}"),
        HandlerError::Business(e) => format!("business write failed: {e}"),
        HandlerError::Commit(e) => format!("commit failed, outcome uncertain: {e}"),
        HandlerError::Escalate {
            reason, event_ids, ..
        } => {
            format!("escalate ({reason}) for events {event_ids:?}")
        }
        HandlerError::RetentionNotAssured { event_ids, .. } => {
            format!("outcome uncertain and retention not assured; reconcile events {event_ids:?}")
        }
    }
}

fn main() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("set DATABASE_URL to run this example against PostgreSQL");
        return;
    };
    let pool = build_pool(&url).expect("pool");
    let plan = Plan {
        operation_id: Uuid::now_v7(),
        // Only true if nothing deletes outbox rows within the retry horizon.
        retention_assured: true,
        events: vec![NewOutboxEnvelope {
            payload: r#"{"order":1}"#.into(),
            metadata: NewOutboxMetadata {
                sequence: None,
                event_id: Uuid::now_v7(),
                message_id: None,
                correlation_id: None,
                causation_id: None,
                event_type: "Order.Placed".into(),
                routing_key: "orders.placed".into(),
                source: None,
                content_type: Some("application/json".into()),
            },
        }],
    };
    let mut history = History::default();
    // A real callback writes business rows keyed by `plan.operation_id`.
    let attempts = NonZeroU32::new(3).expect("non-zero");
    match handle(&pool, &plan, &mut history, attempts, &|_| Ok(())) {
        Ok(()) => println!("recorded"),
        Err(error) => println!("{}", for_subscriber(&error)),
    }
}

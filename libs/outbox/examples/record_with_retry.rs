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
//! * **The callback outcome is recorded.** `transaction` can return a rollback
//!   or commit error in place of the callback's error, so the loop notes what
//!   the callback itself returned and trusts that.
//! * **An uncertain outcome is never resolved by minting new IDs.** The loop
//!   retries the whole transaction with the same plan; if a later attempt
//!   reports a conflict, or retries run out, it stops and escalates, and the
//!   history stays unresolved.
use diesel::prelude::*;
use mulac_outbox::prelude::*;
use uuid::Uuid;

/// Everything a retry must reuse unchanged. Create it once per logical
/// operation, before the first attempt, and persist it.
pub struct Plan {
    pub events: Vec<NewOutboxEnvelope>,
}

/// What earlier attempts established. Persist it with the [`Plan`]: while
/// `unresolved` is set, a later "no rows added" result cannot prove that an
/// earlier attempt added none.
#[derive(Default, Debug)]
pub struct History {
    pub attempts: Vec<Attempt>,
    pub unresolved: bool,
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
        HandlerError::Escalate { .. } => Policy::Stop,
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

fn escalate(plan: &Plan, reason: &'static str, last: HandlerError) -> HandlerError {
    HandlerError::Escalate {
        event_ids: plan.events.iter().map(|e| e.metadata.event_id).collect(),
        reason,
        last: Box::new(last),
    }
}

/// Runs the plan with at most `max_attempts` whole-transaction attempts.
pub fn handle(
    pool: &DbPool,
    plan: &Plan,
    history: &mut History,
    max_attempts: u32,
    business: &dyn Fn(&mut PgConnection) -> QueryResult<()>,
) -> Result<(), HandlerError> {
    let mut last = None;
    for _ in 0..max_attempts {
        let error = match attempt(pool, plan, business) {
            Ok(()) => {
                history.attempts.push(Attempt::Committed);
                // This attempt committed business writes and outbox rows
                // together; the business write must itself be safe to repeat.
                history.unresolved = false;
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
            }
        }
        last = Some(error);
    }
    Err(escalate(
        plan,
        "retries exhausted",
        last.expect("max_attempts must be at least 1"),
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
    }
}

fn main() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("set DATABASE_URL to run this example against PostgreSQL");
        return;
    };
    let pool = build_pool(&url).expect("pool");
    let plan = Plan {
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
    match handle(&pool, &plan, &mut history, 3, &|_| Ok(())) {
        Ok(()) => println!("recorded"),
        Err(error) => println!("{}", for_subscriber(&error)),
    }
}

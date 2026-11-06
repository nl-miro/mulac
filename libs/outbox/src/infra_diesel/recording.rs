//! Atomic batch recording on PostgreSQL.
//!
//! One engine writes a batch on a connection that is already inside a
//! transaction or savepoint. Two wrappers own that boundary: a pooled one that
//! begins and commits its own transaction, and a caller-connection one that
//! only ever uses a savepoint inside the caller's transaction.

use super::entity::NewOutboxEntryRecord;
use super::schema::outbox_entries;
use crate::application::prelude::{
    NewOutboxEntry,
    NewOutboxEnvelope,
    OutboxEntryMetadata,
    RecordingError,
    payload,
    validate_batch,
    //
};

use diesel::connection::{AnsiTransactionManager, SimpleConnection, TransactionManager};
use diesel::pg::PgConnection;
use diesel::prelude::*;
use diesel::sql_types::{Nullable, Text};
use mulac_diesel::DbPool;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Rows per insert statement. Nine bound columns per row keeps a statement far
/// below PostgreSQL's 65,535 bind parameter limit.
const INSERT_CHUNK: usize = 5_000;
const SAVEPOINT: &str = "mulac_outbox_record";
const PROBE_SETTING: &str = "mulac_outbox.probe";

/// Why the engine stopped before the batch was fully recorded.
enum Failure {
    /// A recording-level rejection discovered while writing.
    Rejected(RecordingError),
    Database(diesel::result::Error),
}

impl From<diesel::result::Error> for Failure {
    fn from(error: diesel::result::Error) -> Self {
        Self::Database(error)
    }
}

impl Failure {
    /// The error to report once the batch's writes are known to be rolled back.
    fn rolled_back(self) -> RecordingError {
        match self {
            Self::Rejected(error) => error,
            Self::Database(error) => RecordingError::DatabaseRolledBack(Box::new(error)),
        }
    }
}

/// Owns every transaction-control statement so tests can fail any one step.
struct Session<'c> {
    conn: &'c mut PgConnection,
    #[cfg(test)]
    fail_at: Option<&'static str>,
    #[cfg(test)]
    hooks: Vec<(&'static str, Box<dyn FnMut()>)>,
}

impl<'c> Session<'c> {
    fn new(conn: &'c mut PgConnection) -> Self {
        Self {
            conn,
            #[cfg(test)]
            fail_at: None,
            #[cfg(test)]
            hooks: Vec::new(),
        }
    }

    /// Runs test code at a named point, to create deterministic races.
    fn hook(&mut self, _name: &'static str) {
        #[cfg(test)]
        for (name, run) in &mut self.hooks {
            if *name == _name {
                run();
            }
        }
    }

    fn step(&self, _name: &'static str) -> QueryResult<()> {
        #[cfg(test)]
        if self.fail_at == Some(_name) {
            return Err(diesel::result::Error::DatabaseError(
                diesel::result::DatabaseErrorKind::Unknown,
                Box::new(format!("injected failure at {_name}")),
            ));
        }
        Ok(())
    }

    /// Whether the connection is inside an open transaction, however it was
    /// opened. A transaction-local setting survives to the next statement only
    /// inside a transaction block. This neither reads Diesel's tracked depth nor
    /// interprets error text, and an aborted transaction or lost connection
    /// surfaces as a database error rather than as "no transaction".
    fn in_transaction(&mut self) -> QueryResult<bool> {
        self.step("PROBE")?;
        diesel::sql_query(format!("SELECT set_config('{PROBE_SETTING}', '1', true)"))
            .execute(&mut *self.conn)?;
        let seen = diesel::select(diesel::dsl::sql::<Nullable<Text>>(&format!(
            "current_setting('{PROBE_SETTING}', true)"
        )))
        .get_result::<Option<String>>(&mut *self.conn)?;
        Ok(seen.as_deref() == Some("1"))
    }

    fn savepoint(&mut self) -> QueryResult<()> {
        self.step("SAVEPOINT")?;
        self.conn.batch_execute(&format!("SAVEPOINT {SAVEPOINT}"))
    }

    fn release(&mut self) -> QueryResult<()> {
        self.step("RELEASE")?;
        self.conn
            .batch_execute(&format!("RELEASE SAVEPOINT {SAVEPOINT}"))
    }

    fn rollback_to_savepoint(&mut self) -> QueryResult<()> {
        self.step("ROLLBACK TO")?;
        self.conn
            .batch_execute(&format!("ROLLBACK TO SAVEPOINT {SAVEPOINT}"))
    }

    // The pooled path goes through Diesel's transaction manager so that a failed
    // commit or rollback leaves the connection flagged as broken for the pool.
    fn begin(&mut self) -> QueryResult<()> {
        self.step("BEGIN")?;
        AnsiTransactionManager::begin_transaction(&mut *self.conn)
    }

    fn commit(&mut self) -> QueryResult<()> {
        self.step("COMMIT")?;
        AnsiTransactionManager::commit_transaction(&mut *self.conn)
    }

    fn rollback(&mut self) -> QueryResult<()> {
        self.step("ROLLBACK")?;
        AnsiTransactionManager::rollback_transaction(&mut *self.conn)
    }
}

/// Records a batch on a pooled connection in its own transaction.
pub(super) fn record_pooled(
    pool: &DbPool,
    entries: &[NewOutboxEntry],
) -> Result<(), RecordingError> {
    validate_batch(entries)?;
    if entries.is_empty() {
        return Ok(());
    }
    let mut conn = pool
        .get()
        .map_err(|error| RecordingError::NoConnection(Box::new(error)))?;
    pooled_flow(&mut Session::new(&mut conn), entries)
}

fn pooled_flow(
    session: &mut Session<'_>,
    entries: &[NewOutboxEntry],
) -> Result<(), RecordingError> {
    // Nothing is written until the transaction has begun.
    session
        .begin()
        .map_err(|error| RecordingError::DatabaseRolledBack(Box::new(error)))?;
    match write_batch(session, entries) {
        Ok(()) => {
            session.hook("BEFORE_COMMIT");
            session
                .commit()
                .map_err(|error| RecordingError::DatabaseUncertain(Box::new(error)))
        }
        Err(failure) => match session.rollback() {
            Ok(()) => Err(failure.rolled_back()),
            // A rollback that cannot be confirmed leaves the writes in doubt.
            Err(error) => Err(RecordingError::DatabaseUncertain(Box::new(error))),
        },
    }
}

/// Records a batch on the caller's connection. The caller's transaction is
/// never begun, committed or rolled back here; only a savepoint is used.
pub fn record_batch_on_connection(
    conn: &mut PgConnection,
    envelopes: &[NewOutboxEnvelope],
) -> Result<(), RecordingError> {
    let entries: Vec<_> = envelopes.iter().map(NewOutboxEntry::from).collect();
    validate_batch(&entries)?;
    caller_flow(&mut Session::new(conn), &entries)
}

fn caller_flow(
    session: &mut Session<'_>,
    entries: &[NewOutboxEntry],
) -> Result<(), RecordingError> {
    // A failure to find out whether a transaction is open is a database failure,
    // never "no open transaction". Nothing has been written yet.
    match session.in_transaction() {
        Err(error) => return Err(RecordingError::DatabaseRolledBack(Box::new(error))),
        Ok(false) => return Err(RecordingError::NoOpenTransaction),
        Ok(true) => {}
    }
    if entries.is_empty() {
        return Ok(());
    }
    session
        .savepoint()
        .map_err(|error| RecordingError::DatabaseRolledBack(Box::new(error)))?;
    match write_batch(session, entries) {
        Ok(()) => {
            session.hook("BEFORE_RELEASE");
            session
                .release()
                .map_err(|error| RecordingError::DatabaseUncertain(Box::new(error)))
        }
        Err(failure) => {
            // Roll back to the savepoint so the caller's earlier work survives.
            // If that cannot be confirmed, the writes are in doubt.
            session
                .rollback_to_savepoint()
                .map_err(|error| RecordingError::DatabaseUncertain(Box::new(error)))?;
            // The batch is rolled back; releasing only tidies the savepoint.
            session
                .release()
                .map_err(|error| RecordingError::DatabaseRolledBack(Box::new(error)))?;
            Err(failure.rolled_back())
        }
    }
}

/// Inserts new rows and verifies replays. Event IDs are written in one global
/// order, across statements as well as within them, so batches that hold no
/// other locks cannot wait on each other in a cycle. Replay checks only read
/// committed rows and take no row locks.
fn write_batch(session: &mut Session<'_>, entries: &[NewOutboxEntry]) -> Result<(), Failure> {
    let mut ordered: Vec<&NewOutboxEntry> = entries.iter().collect();
    ordered.sort_unstable_by_key(|entry| entry.id);

    let mut replays: Vec<&NewOutboxEntry> = Vec::new();
    for chunk in ordered.chunks(INSERT_CHUNK) {
        let records: Vec<NewOutboxEntryRecord> = chunk
            .iter()
            .map(|entry| NewOutboxEntryRecord::from(*entry))
            .collect();
        let inserted: HashSet<Uuid> = diesel::insert_into(outbox_entries::table)
            .values(&records)
            .on_conflict(outbox_entries::id)
            .do_nothing()
            .returning(outbox_entries::id)
            .get_results::<Uuid>(&mut *session.conn)?
            .into_iter()
            .collect();
        replays.extend(
            chunk
                .iter()
                .copied()
                .filter(|entry| !inserted.contains(&entry.id)),
        );
    }
    session.hook("AFTER_INSERT");
    verify_replays(&mut *session.conn, &replays)
}

fn verify_replays(conn: &mut PgConnection, replays: &[&NewOutboxEntry]) -> Result<(), Failure> {
    let mut missing = Vec::new();
    for chunk in replays.chunks(INSERT_CHUNK) {
        let ids: Vec<Uuid> = chunk.iter().map(|entry| entry.id).collect();
        let stored: HashMap<Uuid, (String, String)> = outbox_entries::table
            .filter(outbox_entries::id.eq_any(&ids))
            .select((
                outbox_entries::id,
                outbox_entries::payload,
                diesel::dsl::sql::<Text>("meta::text"),
            ))
            .load::<(Uuid, String, String)>(conn)?
            .into_iter()
            .map(|(id, body, meta)| (id, (body, meta)))
            .collect();
        for entry in chunk {
            let Some((body, meta)) = stored.get(&entry.id) else {
                missing.push(entry.id);
                continue;
            };
            if let Some(field) = differing_field(entry, body, meta) {
                return Err(Failure::Rejected(RecordingError::ReplayConflict {
                    event_id: entry.id,
                    field,
                }));
            }
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(Failure::Rejected(RecordingError::MissingEvents(missing)))
    }
}

/// The first compared field on which a stored row differs from the new entry,
/// or a stored-row defect. Correlation, causation and source IDs, timestamps
/// and delivery state are deliberately not compared.
fn differing_field(entry: &NewOutboxEntry, body: &str, meta: &str) -> Option<&'static str> {
    let Ok(stored) = serde_json::from_str::<OutboxEntryMetadata>(meta) else {
        return Some("stored_metadata");
    };
    let sent = &entry.meta;
    if stored.event_id != entry.id {
        Some("stored_event_id")
    } else if payload(body).ok() != payload(&entry.payload).ok() {
        Some("payload")
    } else if stored.message_id != sent.message_id {
        Some("message_id")
    } else if stored.routing_key != sent.routing_key {
        Some("routing_key")
    } else if stored.event_type != sent.event_type {
        Some("event_type")
    } else if stored.content_type != sent.content_type {
        Some("content_type")
    } else if stored.sequence != sent.sequence {
        Some("sequence")
    } else {
        None
    }
}

#[cfg(test)]
mod fault_tests {
    //! Fault injection against real PostgreSQL. Each case fails exactly one
    //! transaction-control step and asserts the reported outcome and the error
    //! that step observed.
    use super::*;
    use crate::application::prelude::RecordingOutcome;
    use crate::test_common::{TestDb, count, entry, ids};
    use diesel::r2d2::R2D2Connection;
    use std::error::Error;

    fn source_text(error: &RecordingError) -> String {
        error.source().map(ToString::to_string).unwrap_or_default()
    }

    fn caller(
        conn: &mut PgConnection,
        fail_at: &'static str,
        entries: &[NewOutboxEntry],
    ) -> Result<(), RecordingError> {
        let mut session = Session::new(conn);
        session.fail_at = Some(fail_at);
        caller_flow(&mut session, entries)
    }

    fn conflicting(db: &TestDb) -> Vec<NewOutboxEntry> {
        let existing = entry(Uuid::now_v7());
        let mut stored = existing.clone();
        stored.payload = r#"{"stored":true}"#.into();
        crate::prelude::OutboxStorePort::record(&db.repo(), &stored).unwrap();
        vec![entry(Uuid::now_v7()), existing]
    }

    #[test]
    #[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
    fn failed_eligibility_probe_is_a_database_failure_not_no_open_transaction() {
        let db = TestDb::new();
        let mut conn = db.conn();
        // Idle session: without the injected failure this would be NoOpenTransaction.
        let err = caller(&mut conn, "PROBE", &[entry(Uuid::now_v7())]).unwrap_err();
        assert!(
            matches!(err, RecordingError::DatabaseRolledBack(_)),
            "{err:?}"
        );
        assert!(source_text(&err).contains("injected failure at PROBE"));
        assert_eq!(err.outcome(), RecordingOutcome::NoRowsAdded);
    }

    #[test]
    #[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
    fn failed_savepoint_establishment_adds_no_rows_and_retains_the_step_error() {
        let db = TestDb::new();
        let mut conn = db.conn();
        conn.batch_execute("BEGIN").unwrap();
        let err = caller(&mut conn, "SAVEPOINT", &[entry(Uuid::now_v7())]).unwrap_err();
        assert!(
            matches!(err, RecordingError::DatabaseRolledBack(_)),
            "{err:?}"
        );
        assert!(source_text(&err).contains("injected failure at SAVEPOINT"));
        assert_eq!(count(&mut conn), 0);
    }

    #[test]
    #[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
    fn failed_release_after_success_is_uncertain_with_the_release_error() {
        let db = TestDb::new();
        let mut conn = db.conn();
        conn.batch_execute("BEGIN").unwrap();
        let err = caller(&mut conn, "RELEASE", &[entry(Uuid::now_v7())]).unwrap_err();
        assert!(
            matches!(err, RecordingError::DatabaseUncertain(_)),
            "{err:?}"
        );
        assert!(source_text(&err).contains("injected failure at RELEASE"));
        assert_eq!(err.outcome(), RecordingOutcome::Uncertain);
    }

    #[test]
    #[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
    fn failed_rollback_after_a_healthy_conflict_is_uncertain_with_only_the_rollback_error() {
        let db = TestDb::new();
        let batch = conflicting(&db);
        let mut conn = db.conn();
        conn.batch_execute("BEGIN").unwrap();
        let err = caller(&mut conn, "ROLLBACK TO", &batch).unwrap_err();
        assert!(
            matches!(err, RecordingError::DatabaseUncertain(_)),
            "{err:?}"
        );
        assert!(source_text(&err).contains("injected failure at ROLLBACK TO"));
    }

    #[test]
    #[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
    fn failed_release_after_a_confirmed_rollback_still_reports_no_rows_added() {
        let db = TestDb::new();
        let batch = conflicting(&db);
        let mut conn = db.conn();
        conn.batch_execute("BEGIN").unwrap();
        let err = caller(&mut conn, "RELEASE", &batch).unwrap_err();
        assert!(
            matches!(err, RecordingError::DatabaseRolledBack(_)),
            "{err:?}"
        );
        assert!(source_text(&err).contains("injected failure at RELEASE"));
        // The new row of the failed batch was rolled back.
        assert_eq!(count(&mut conn), 1);
    }

    /// A trigger makes the highest event ID fail, so with more than one insert
    /// chunk an earlier chunk has genuinely succeeded before the failure.
    fn failing_tail_batch(db: &TestDb) -> Vec<NewOutboxEntry> {
        let mut batch: Vec<_> = ids(INSERT_CHUNK + 10).into_iter().map(entry).collect();
        let last = batch.iter().map(|e| e.id).max().unwrap();
        db.exec(&format!(
            "CREATE FUNCTION fail_tail() RETURNS trigger LANGUAGE plpgsql AS $$ \
             BEGIN IF NEW.id = '{last}' THEN RAISE EXCEPTION 'forced failure after % rows', (SELECT count(*) FROM outbox_entries); END IF; RETURN NEW; END $$; \
             CREATE TRIGGER fail_tail BEFORE INSERT ON outbox_entries \
             FOR EACH ROW EXECUTE FUNCTION fail_tail();"
        ));
        batch.reverse();
        batch
    }

    fn rows_before_failure(error: &RecordingError) -> usize {
        source_text(error)
            .split("forced failure after ")
            .nth(1)
            .and_then(|rest| rest.split(' ').next())
            .and_then(|n| n.parse().ok())
            .expect("trigger message with row count")
    }

    #[test]
    #[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
    fn database_failure_after_an_earlier_chunk_succeeded_rolls_everything_back() {
        let db = TestDb::new();
        let batch = failing_tail_batch(&db);

        // Pooled: own transaction.
        let err = super::record_pooled(&db.pool(), &batch).unwrap_err();
        assert!(
            matches!(err, RecordingError::DatabaseRolledBack(_)),
            "{err:?}"
        );
        // The trigger reports the rows visible to the failing transaction, which
        // proves an earlier insertion chunk had really been written.
        assert!(rows_before_failure(&err) >= INSERT_CHUNK, "{err:?}");
        assert_eq!(db.count(), 0);

        // Caller connection: savepoint; the caller's earlier work survives.
        let mut conn = db.conn();
        conn.batch_execute("BEGIN; CREATE TABLE keep (n INT)")
            .unwrap();
        let err = caller_flow(&mut Session::new(&mut conn), &batch).unwrap_err();
        assert!(
            matches!(err, RecordingError::DatabaseRolledBack(_)),
            "{err:?}"
        );
        assert_eq!(count(&mut conn), 0);
        conn.batch_execute("INSERT INTO keep VALUES (1); COMMIT")
            .unwrap();
    }

    fn pooled(
        db: &TestDb,
        fail_at: &'static str,
        entries: &[NewOutboxEntry],
    ) -> (Result<(), RecordingError>, bool) {
        let pool = db.pool();
        let mut conn = pool.get().unwrap();
        let mut session = Session::new(&mut conn);
        session.fail_at = Some(fail_at);
        let result = pooled_flow(&mut session, entries);
        let broken = conn.is_broken();
        (result, broken)
    }

    #[test]
    #[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
    fn pooled_begin_commit_and_rollback_failures_are_classified_by_phase() {
        let db = TestDb::new();

        let (result, _) = pooled(&db, "BEGIN", &[entry(Uuid::now_v7())]);
        let err = result.unwrap_err();
        assert!(
            matches!(err, RecordingError::DatabaseRolledBack(_)),
            "{err:?}"
        );

        let (result, broken) = pooled(&db, "COMMIT", &[entry(Uuid::now_v7())]);
        let err = result.unwrap_err();
        assert!(
            matches!(err, RecordingError::DatabaseUncertain(_)),
            "{err:?}"
        );
        assert!(source_text(&err).contains("injected failure at COMMIT"));
        assert!(
            broken,
            "a connection with an unconfirmed commit must not be reused"
        );

        let batch = conflicting(&db);
        let (result, broken) = pooled(&db, "ROLLBACK", &batch);
        let err = result.unwrap_err();
        assert!(
            matches!(err, RecordingError::DatabaseUncertain(_)),
            "{err:?}"
        );
        assert!(source_text(&err).contains("injected failure at ROLLBACK"));
        assert!(
            broken,
            "a connection with an unconfirmed rollback must not be reused"
        );
    }

    fn deleter(db: &TestDb, id: Uuid) -> Box<dyn FnMut()> {
        let url = db.url();
        Box::new(move || {
            let mut other = PgConnection::establish(&url).unwrap();
            other
                .batch_execute(&format!("DELETE FROM outbox_entries WHERE id = '{id}'"))
                .unwrap();
        })
    }

    fn committed(db: &TestDb) -> NewOutboxEntry {
        let existing = entry(Uuid::now_v7());
        crate::prelude::OutboxStorePort::record(&db.repo(), &existing).unwrap();
        existing
    }

    #[test]
    #[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
    fn row_deleted_between_insert_and_verification_is_missing_and_retry_inserts_again() {
        let db = TestDb::new();
        let existing = committed(&db);
        let fresh = entry(Uuid::now_v7());
        let batch = vec![fresh.clone(), existing.clone()];

        let pool = db.pool();
        let mut conn = pool.get().unwrap();
        let mut session = Session::new(&mut conn);
        session
            .hooks
            .push(("AFTER_INSERT", deleter(&db, existing.id)));
        let err = pooled_flow(&mut session, &batch).unwrap_err();
        drop(conn);
        assert!(
            matches!(&err, RecordingError::MissingEvents(ids) if ids == &[existing.id]),
            "{err:?}"
        );
        assert_eq!(err.outcome(), RecordingOutcome::NoRowsAdded);
        assert_eq!(db.count(), 0, "the new row was rolled back with the batch");

        // Replay protection lasts only while the row is stored: the retry inserts both.
        crate::prelude::OutboxStorePort::record_batch(&db.repo(), &batch).unwrap();
        assert_eq!(db.count(), 2);
    }

    #[test]
    #[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
    fn row_deleted_after_verification_still_reports_success_at_observation_time() {
        let db = TestDb::new();
        let existing = committed(&db);
        let fresh = entry(Uuid::now_v7());
        let batch = vec![fresh.clone(), existing.clone()];

        // Pooled: deleted before commit; success describes the moment of observation.
        let pool = db.pool();
        let mut conn = pool.get().unwrap();
        let mut session = Session::new(&mut conn);
        session
            .hooks
            .push(("BEFORE_COMMIT", deleter(&db, existing.id)));
        pooled_flow(&mut session, &batch).expect("success");
        drop(conn);
        assert_eq!(
            db.count(),
            1,
            "only the new row remains; this is not a partial commit"
        );

        // Caller connection: deleted before the savepoint is released.
        let existing = committed(&db);
        let mut conn = db.conn();
        conn.batch_execute("BEGIN").unwrap();
        let mut session = Session::new(&mut conn);
        session
            .hooks
            .push(("BEFORE_RELEASE", deleter(&db, existing.id)));
        caller_flow(&mut session, &[entry(Uuid::now_v7()), existing.clone()]).expect("success");
        conn.batch_execute("COMMIT").unwrap();
        assert!(crate::test_common::stored(&mut conn, existing.id).is_none());
    }
}

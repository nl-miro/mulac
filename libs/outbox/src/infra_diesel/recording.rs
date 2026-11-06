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
}

impl<'c> Session<'c> {
    fn new(conn: &'c mut PgConnection) -> Self {
        Self {
            conn,
            #[cfg(test)]
            fail_at: None,
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
    match write_batch(session.conn, entries) {
        Ok(()) => session
            .commit()
            .map_err(|error| RecordingError::DatabaseUncertain(Box::new(error))),
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
    match write_batch(session.conn, entries) {
        Ok(()) => session
            .release()
            .map_err(|error| RecordingError::DatabaseUncertain(Box::new(error))),
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
fn write_batch(conn: &mut PgConnection, entries: &[NewOutboxEntry]) -> Result<(), Failure> {
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
            .get_results::<Uuid>(conn)?
            .into_iter()
            .collect();
        replays.extend(
            chunk
                .iter()
                .copied()
                .filter(|entry| !inserted.contains(&entry.id)),
        );
    }
    verify_replays(conn, &replays)
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

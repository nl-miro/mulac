pub mod io {
    pub use super::storage::{DbPool, OutboxConsumerStorage, OutboxStoreStorage, build_pool};
}

mod models {
    use diesel::deserialize::{FromSql, Result as DeserializeResult};
    use diesel::pg::{Pg, PgValue};
    use diesel::serialize::{IsNull, Output, Result as SerializeResult, ToSql};
    use diesel::sql_types::Jsonb;
    use diesel::{AsExpression, FromSqlRow};
    use serde_json::{from_slice, to_writer};
    use std::io::Write;

    use crate::assembly::domain::OutboxEntryMetadata;

    #[derive(Debug, Clone, PartialEq, Eq, AsExpression, FromSqlRow)]
    #[diesel(sql_type = Jsonb)]
    pub struct MetadataJsonb(pub OutboxEntryMetadata);

    impl ToSql<Jsonb, Pg> for MetadataJsonb {
        fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, Pg>) -> SerializeResult {
            out.write_all(&[1])?;
            to_writer(out, &self.0)?;
            Ok(IsNull::No)
        }
    }

    impl FromSql<Jsonb, Pg> for MetadataJsonb {
        fn from_sql(bytes: PgValue<'_>) -> DeserializeResult<Self> {
            let bytes = bytes.as_bytes();
            if bytes.is_empty() {
                return Err("empty jsonb value".into());
            }

            if bytes[0] != 1 {
                return Err(format!("unsupported jsonb version: {}", bytes[0]).into());
            }

            Ok(MetadataJsonb(from_slice(&bytes[1..])?))
        }
    }
}

mod conversions {
    use crate::assembly::application::io::{OutboxEntryEnvelope, OutboxError};
    use crate::assembly::domain::{
        NewOutboxEntry as DomainNewOutboxEntry, OutboxEntry as DomainOutboxEntry, OutboxStatus,
    };

    use super::entity::{NewOutboxEntryRecord, OutboxEntryRecord};
    use super::models::MetadataJsonb;

    impl From<&DomainNewOutboxEntry> for NewOutboxEntryRecord {
        fn from(entry: &DomainNewOutboxEntry) -> Self {
            Self::new(
                entry.id,
                entry.payload.clone(),
                MetadataJsonb(entry.meta.clone()),
                OutboxStatus::Received as i32,
                entry.scheduled_at,
                0,
                entry.received_at,
                entry.received_at,
            )
        }
    }

    impl TryFrom<OutboxEntryRecord> for OutboxEntryEnvelope {
        type Error = OutboxError;

        fn try_from(record: OutboxEntryRecord) -> Result<Self, Self::Error> {
            let status = OutboxStatus::try_from(record.status())
                .map_err(|err| OutboxError::Storage(err.to_string()))?;
            let metadata = record.meta().clone();

            Ok(OutboxEntryEnvelope {
                message: DomainOutboxEntry {
                    id: record.id(),
                    status,
                    payload: record.payload().to_string(),
                    meta: metadata.clone(),
                    scheduled_at: record.scheduled_at(),
                    attempts: record.attempts(),
                    reservation_id: record.reservation_id(),
                    reserved_at: record.reserved_at(),
                    received_at: record.received_at(),
                    updated_at: record.updated_at(),
                    processed_at: record.processed_at(),
                    last_error: record.last_error().map(str::to_owned),
                },
                metadata,
            })
        }
    }
}

pub(crate) mod schema {
    diesel::table! {
        outbox_entries (id) {
            id -> Uuid,
            status -> Int4,
            payload -> Text,
            meta -> Jsonb,
            scheduled_at -> Timestamptz,
            attempts -> Int4,
            reservation_id -> Nullable<Uuid>,
            reserved_at -> Nullable<Timestamptz>,
            received_at -> Timestamptz,
            updated_at -> Timestamptz,
            processed_at -> Nullable<Timestamptz>,
            last_error -> Nullable<Text>,
        }
    }
}

pub(crate) mod entity {
    use chrono::{DateTime, Utc};
    use diesel::{Insertable, Queryable, QueryableByName, Selectable};
    use uuid::Uuid;

    use crate::assembly::domain::OutboxEntryMetadata;

    use super::models::MetadataJsonb;

    #[derive(Debug, Insertable)]
    #[diesel(table_name = super::schema::outbox_entries)]
    pub struct NewOutboxEntryRecord {
        id: Uuid,
        status: i32,
        payload: String,
        meta: MetadataJsonb,
        scheduled_at: DateTime<Utc>,
        attempts: i32,
        received_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    }

    impl NewOutboxEntryRecord {
        pub(super) fn new(
            id: Uuid,
            payload: String,
            meta: MetadataJsonb,
            status: i32,
            scheduled_at: DateTime<Utc>,
            attempts: i32,
            received_at: DateTime<Utc>,
            updated_at: DateTime<Utc>,
        ) -> Self {
            Self {
                id,
                status,
                payload,
                meta,
                scheduled_at,
                attempts,
                received_at,
                updated_at,
            }
        }

        pub fn id(&self) -> Uuid {
            self.id
        }

        pub fn status(&self) -> i32 {
            self.status
        }

        pub fn payload(&self) -> &str {
            &self.payload
        }

        pub fn meta(&self) -> &OutboxEntryMetadata {
            &self.meta.0
        }

        pub fn scheduled_at(&self) -> DateTime<Utc> {
            self.scheduled_at
        }

        pub fn attempts(&self) -> i32 {
            self.attempts
        }

        pub fn received_at(&self) -> DateTime<Utc> {
            self.received_at
        }

        pub fn updated_at(&self) -> DateTime<Utc> {
            self.updated_at
        }
    }

    #[derive(Debug, Queryable, QueryableByName, Selectable)]
    #[diesel(table_name = super::schema::outbox_entries)]
    pub struct OutboxEntryRecord {
        id: Uuid,
        status: i32,
        payload: String,
        meta: MetadataJsonb,
        scheduled_at: DateTime<Utc>,
        attempts: i32,
        reservation_id: Option<Uuid>,
        reserved_at: Option<DateTime<Utc>>,
        received_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
        processed_at: Option<DateTime<Utc>>,
        last_error: Option<String>,
    }

    impl OutboxEntryRecord {
        #[cfg(test)]
        #[allow(clippy::too_many_arguments)]
        pub(super) fn new(
            id: Uuid,
            status: i32,
            payload: String,
            meta: MetadataJsonb,
            scheduled_at: DateTime<Utc>,
            attempts: i32,
            reservation_id: Option<Uuid>,
            reserved_at: Option<DateTime<Utc>>,
            received_at: DateTime<Utc>,
            updated_at: DateTime<Utc>,
            processed_at: Option<DateTime<Utc>>,
            last_error: Option<String>,
        ) -> Self {
            Self {
                id,
                status,
                payload,
                meta,
                scheduled_at,
                attempts,
                reservation_id,
                reserved_at,
                received_at,
                updated_at,
                processed_at,
                last_error,
            }
        }

        pub fn id(&self) -> Uuid {
            self.id
        }

        pub fn status(&self) -> i32 {
            self.status
        }

        pub fn payload(&self) -> &str {
            &self.payload
        }

        pub fn meta(&self) -> &OutboxEntryMetadata {
            &self.meta.0
        }

        pub fn scheduled_at(&self) -> DateTime<Utc> {
            self.scheduled_at
        }

        pub fn attempts(&self) -> i32 {
            self.attempts
        }

        pub fn reservation_id(&self) -> Option<Uuid> {
            self.reservation_id
        }

        pub fn reserved_at(&self) -> Option<DateTime<Utc>> {
            self.reserved_at
        }

        pub fn received_at(&self) -> DateTime<Utc> {
            self.received_at
        }

        pub fn updated_at(&self) -> DateTime<Utc> {
            self.updated_at
        }

        pub fn processed_at(&self) -> Option<DateTime<Utc>> {
            self.processed_at
        }

        pub fn last_error(&self) -> Option<&str> {
            self.last_error.as_deref()
        }
    }
}

mod storage {
    use chrono::{DateTime, Duration, Utc};
    use diesel::PgConnection;
    use diesel::prelude::*;
    use diesel::r2d2::{ConnectionManager, Pool};
    use diesel::sql_types::{Array, BigInt, Int4, Uuid as SqlUuid};
    use uuid::Uuid;

    use crate::assembly::application::io::{
        OutboxEntryEnvelope, OutboxError, OutboxProcessPort, OutboxReservePort, OutboxStorePort,
        OutboxSweepPort,
    };
    use crate::assembly::domain::NewOutboxEntry;
    use crate::assembly::domain::OutboxStatus;
    use crate::outbox_consumer::io::ReservableOutboxSpec;
    use crate::stale_reservation_sweep::io::StaleReservationSpec;

    use super::entity::{NewOutboxEntryRecord, OutboxEntryRecord};
    use super::schema::outbox_entries;

    pub type DbPool = Pool<ConnectionManager<PgConnection>>;

    pub fn build_pool(database_url: &str) -> Result<DbPool, diesel::r2d2::PoolError> {
        let manager = ConnectionManager::<PgConnection>::new(database_url);
        Pool::builder().build(manager)
    }

    pub struct OutboxStoreStorage {
        pub(crate) pool: DbPool,
    }

    impl OutboxStoreStorage {
        pub fn new(pool: DbPool) -> Self {
            Self { pool }
        }
    }

    impl OutboxStorePort for OutboxStoreStorage {
        fn record(&self, entry: &NewOutboxEntry) -> Result<(), OutboxError> {
            let mut conn = self
                .pool
                .get()
                .map_err(|err| OutboxError::Storage(err.to_string()))?;
            let record = NewOutboxEntryRecord::from(entry);

            diesel::insert_into(outbox_entries::table)
                .values(&record)
                .on_conflict(outbox_entries::id)
                .do_nothing()
                .execute(&mut conn)
                .map_err(|err| OutboxError::Storage(err.to_string()))?;

            Ok(())
        }
    }

    pub struct OutboxConsumerStorage {
        pub(crate) pool: DbPool,
    }

    impl OutboxConsumerStorage {
        pub(crate) const RETRY_BACKOFF_SECONDS: i64 = 30;
        pub(crate) const RETRY_BACKOFF_CAP_SECONDS: i64 = 120;

        pub fn new(pool: DbPool) -> Self {
            Self { pool }
        }
    }

    impl OutboxReservePort for OutboxConsumerStorage {
        fn reserve(
            &self,
            spec: &ReservableOutboxSpec,
        ) -> Result<Vec<OutboxEntryEnvelope>, OutboxError> {
            let mut conn = self
                .pool
                .get()
                .map_err(|err| OutboxError::Storage(err.to_string()))?;
            let reservation_id = Uuid::now_v7();

            let entries = diesel::sql_query(
                r#"
                WITH candidates AS (
                    SELECT id
                    FROM outbox_entries
                    WHERE status = ANY($1)
                      AND scheduled_at <= now()
                      AND attempts < $2
                    ORDER BY scheduled_at ASC
                    LIMIT $3
                    FOR UPDATE SKIP LOCKED
                )
                UPDATE outbox_entries AS entries
                SET status = $4,
                    reservation_id = $5,
                    reserved_at = now(),
                    attempts = entries.attempts + 1,
                    updated_at = now()
                FROM candidates
                WHERE entries.id = candidates.id
                RETURNING
                    entries.id,
                    entries.status,
                    entries.payload,
                    entries.meta,
                    entries.scheduled_at,
                    entries.attempts,
                    entries.reservation_id,
                    entries.reserved_at,
                    entries.received_at,
                    entries.updated_at,
                    entries.processed_at,
                    entries.last_error
                "#,
            )
            .bind::<Array<Int4>, _>(vec![
                i32::from(OutboxStatus::Received),
                i32::from(OutboxStatus::Failed),
            ])
            .bind::<Int4, _>(spec.max_attempts)
            .bind::<BigInt, _>(spec.limit as i64)
            .bind::<Int4, _>(i32::from(OutboxStatus::Reserved))
            .bind::<SqlUuid, _>(reservation_id)
            .load::<OutboxEntryRecord>(&mut conn)
            .map_err(|err| OutboxError::Storage(err.to_string()))?;

            entries
                .into_iter()
                .map(OutboxEntryEnvelope::try_from)
                .collect()
        }
    }

    impl OutboxProcessPort for OutboxConsumerStorage {
        fn completed(&self, id: Uuid, reservation_id: Uuid) -> Result<(), OutboxError> {
            let mut conn = self
                .pool
                .get()
                .map_err(|err| OutboxError::Storage(err.to_string()))?;

            let updated = diesel::update(
                outbox_entries::table
                    .filter(outbox_entries::id.eq(id))
                    .filter(outbox_entries::reservation_id.eq(reservation_id))
                    .filter(outbox_entries::status.eq(i32::from(OutboxStatus::Reserved))),
            )
            .set((
                outbox_entries::status.eq(i32::from(OutboxStatus::Completed)),
                outbox_entries::processed_at.eq(diesel::dsl::now),
                outbox_entries::updated_at.eq(diesel::dsl::now),
                outbox_entries::reservation_id.eq(None::<Uuid>),
                outbox_entries::reserved_at.eq(None::<DateTime<Utc>>),
                outbox_entries::last_error.eq(None::<String>),
            ))
            .execute(&mut conn)
            .map_err(|err| OutboxError::Storage(err.to_string()))?;

            if updated == 0 {
                return Err(reservation_not_owned(id, reservation_id));
            }

            Ok(())
        }

        fn failed(
            &self,
            id: Uuid,
            reservation_id: Uuid,
            max_attempts: i32,
            reason: Option<String>,
        ) -> Result<(), OutboxError> {
            let mut conn = self
                .pool
                .get()
                .map_err(|err| OutboxError::Storage(err.to_string()))?;

            conn.transaction::<(), diesel::result::Error, _>(|conn| {
                let attempts = diesel::update(
                    outbox_entries::table
                        .filter(outbox_entries::id.eq(id))
                        .filter(outbox_entries::reservation_id.eq(reservation_id))
                        .filter(outbox_entries::status.eq(i32::from(OutboxStatus::Reserved))),
                )
                .set((
                    outbox_entries::reservation_id.eq(None::<Uuid>),
                    outbox_entries::reserved_at.eq(None::<DateTime<Utc>>),
                    outbox_entries::updated_at.eq(diesel::dsl::now),
                ))
                .returning(outbox_entries::attempts)
                .get_result::<i32>(conn)?;

                let exhausted = attempts >= max_attempts;
                let status = if exhausted {
                    OutboxStatus::Dead
                } else {
                    OutboxStatus::Failed
                };
                let now = Utc::now();
                let retry_delay_seconds = retry_delay_seconds(attempts);

                if exhausted {
                    diesel::update(outbox_entries::table.find(id))
                        .set((
                            outbox_entries::status.eq(i32::from(status)),
                            outbox_entries::updated_at.eq(now),
                            outbox_entries::last_error.eq(reason.clone()),
                        ))
                        .execute(conn)?;
                } else {
                    diesel::update(outbox_entries::table.find(id))
                        .set((
                            outbox_entries::status.eq(i32::from(status)),
                            outbox_entries::scheduled_at
                                .eq(now + Duration::seconds(retry_delay_seconds)),
                            outbox_entries::updated_at.eq(now),
                            outbox_entries::last_error.eq(reason.clone()),
                        ))
                        .execute(conn)?;
                }

                Ok(())
            })
            .map_err(|err| match err {
                diesel::result::Error::NotFound => reservation_not_owned(id, reservation_id),
                other => OutboxError::Storage(other.to_string()),
            })?;

            Ok(())
        }

        fn dead(
            &self,
            id: Uuid,
            reservation_id: Uuid,
            reason: Option<String>,
        ) -> Result<(), OutboxError> {
            let mut conn = self
                .pool
                .get()
                .map_err(|err| OutboxError::Storage(err.to_string()))?;

            let updated = diesel::update(
                outbox_entries::table
                    .filter(outbox_entries::id.eq(id))
                    .filter(outbox_entries::reservation_id.eq(reservation_id))
                    .filter(outbox_entries::status.eq(i32::from(OutboxStatus::Reserved))),
            )
            .set((
                outbox_entries::status.eq(i32::from(OutboxStatus::Dead)),
                outbox_entries::updated_at.eq(diesel::dsl::now),
                outbox_entries::reservation_id.eq(None::<Uuid>),
                outbox_entries::reserved_at.eq(None::<DateTime<Utc>>),
                outbox_entries::last_error.eq(reason),
            ))
            .execute(&mut conn)
            .map_err(|err| OutboxError::Storage(err.to_string()))?;

            if updated == 0 {
                return Err(reservation_not_owned(id, reservation_id));
            }

            Ok(())
        }
    }

    impl OutboxSweepPort for OutboxConsumerStorage {
        fn sweep(&self, spec: &StaleReservationSpec) -> Result<u64, OutboxError> {
            let mut conn = self
                .pool
                .get()
                .map_err(|err| OutboxError::Storage(err.to_string()))?;
            let now = Utc::now();
            let cutoff = now - spec.timeout;

            let affected = diesel::sql_query(
                r#"
                UPDATE outbox_entries
                SET
                    status = CASE
                        WHEN attempts >= $1 THEN $2
                        ELSE $3
                    END,
                    reservation_id = NULL,
                    reserved_at = NULL,
                    scheduled_at = CASE
                        WHEN attempts >= $1 THEN scheduled_at
                        ELSE $4 + LEAST(GREATEST(attempts, 1) * $5, $6) * interval '1 second'
                    END,
                    updated_at = $4
                WHERE status = $7
                  AND reserved_at IS NOT NULL
                  AND reserved_at < $8
                "#,
            )
            .bind::<Int4, _>(spec.max_attempts)
            .bind::<Int4, _>(i32::from(OutboxStatus::Dead))
            .bind::<Int4, _>(i32::from(OutboxStatus::Failed))
            .bind::<diesel::sql_types::Timestamptz, _>(now)
            .bind::<BigInt, _>(Self::RETRY_BACKOFF_SECONDS)
            .bind::<BigInt, _>(Self::RETRY_BACKOFF_CAP_SECONDS)
            .bind::<Int4, _>(i32::from(OutboxStatus::Reserved))
            .bind::<diesel::sql_types::Timestamptz, _>(cutoff)
            .execute(&mut conn)
            .map_err(|err| OutboxError::Storage(err.to_string()))?;

            Ok(affected as u64)
        }
    }

    fn reservation_not_owned(id: Uuid, reservation_id: Uuid) -> OutboxError {
        OutboxError::Reservation(format!(
            "outbox entry {id} is not reserved by reservation {reservation_id}"
        ))
    }

    fn retry_delay_seconds(attempts: i32) -> i64 {
        (i64::from(attempts.max(1)) * OutboxConsumerStorage::RETRY_BACKOFF_SECONDS)
            .min(OutboxConsumerStorage::RETRY_BACKOFF_CAP_SECONDS)
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use uuid::Uuid;

    use crate::assembly::application::io::OutboxEntryEnvelope;
    use crate::assembly::domain::{NewOutboxEntry, OutboxEntryMetadata, OutboxStatus};

    use super::entity::{NewOutboxEntryRecord, OutboxEntryRecord};
    use super::models::MetadataJsonb;

    fn metadata(event_id: Uuid) -> OutboxEntryMetadata {
        OutboxEntryMetadata {
            event_id,
            message_id: event_id,
            correlation_id: None,
            causation_id: None,
            event_type: "UserRegistered".into(),
            routing_key: "users.registered".into(),
            source: Some("identity-service".into()),
            content_type: Some("application/json".into()),
        }
    }

    #[test]
    fn new_outbox_entry_converts_to_insertable_record() {
        let event_id = Uuid::now_v7();
        let now = Utc::now();
        let entry = NewOutboxEntry {
            id: event_id,
            payload: "{}".into(),
            meta: metadata(event_id),
            scheduled_at: now,
            received_at: now,
        };

        let record = NewOutboxEntryRecord::from(&entry);

        assert_eq!(record.id(), event_id);
        assert_eq!(record.status(), OutboxStatus::Received as i32);
        assert_eq!(record.payload(), "{}");
        assert_eq!(record.meta().event_id, event_id);
        assert_eq!(record.attempts(), 0);
        assert_eq!(record.scheduled_at(), now);
        assert_eq!(record.received_at(), now);
        assert_eq!(record.updated_at(), now);
    }

    #[test]
    fn outbox_record_converts_to_entry_envelope() {
        let event_id = Uuid::now_v7();
        let reservation_id = Uuid::now_v7();
        let now = Utc::now();
        let record = OutboxEntryRecord::new(
            event_id,
            OutboxStatus::Reserved as i32,
            "{}".into(),
            MetadataJsonb(metadata(event_id)),
            now,
            2,
            Some(reservation_id),
            Some(now),
            now,
            now,
            None,
            Some("broker unavailable".into()),
        );

        let envelope = OutboxEntryEnvelope::try_from(record).expect("record converts");

        assert_eq!(envelope.message.id, event_id);
        assert_eq!(envelope.message.status, OutboxStatus::Reserved);
        assert_eq!(envelope.message.attempts, 2);
        assert_eq!(envelope.message.reservation_id, Some(reservation_id));
        assert_eq!(
            envelope.message.last_error.as_deref(),
            Some("broker unavailable")
        );
        assert_eq!(envelope.metadata.event_id, event_id);
    }
}

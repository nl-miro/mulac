pub mod io {
    pub use super::entity::{EventEntry, NewEventEntry};
    pub use super::storage::{EventConsumerStorage, EventStoreStorage};
}

pub(crate) mod models {
    use diesel::deserialize::{FromSql, Result as DeserializeResult};
    use diesel::pg::{Pg, PgValue};
    use diesel::serialize::{IsNull, Output, Result as SerializeResult, ToSql};
    use diesel::sql_types::Jsonb;
    use diesel::{AsExpression, FromSqlRow};
    use serde_json::{Value, from_slice, to_writer};
    use std::io::Write;

    #[derive(Debug, Clone, PartialEq, AsExpression, FromSqlRow)]
    #[diesel(sql_type = Jsonb)]
    pub struct MetadataJsonb(pub Value);

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
    use super::entity::{EventEntry, NewEventEntry};
    use super::models::MetadataJsonb;
    use crate::assembly::application::io::{
        EventEnvelope, EventError, EventMetadata, NewEventEnvelope, NewEventMetadata,
    };
    use chrono::Utc;

    impl TryFrom<&NewEventEnvelope> for NewEventEntry {
        type Error = EventError;

        fn try_from(envelope: &NewEventEnvelope) -> Result<Self, Self::Error> {
            let metadata = envelope.metadata.as_ref().ok_or_else(|| {
                EventError::Conversion("event_id is required: metadata is missing".into())
            })?;

            let now = Utc::now();
            let meta = serde_json::to_value(metadata).ok().map(MetadataJsonb);

            Ok(NewEventEntry {
                id: metadata.event_id,
                event_type: envelope.event_type.clone(),
                payload: envelope.payload.clone(),
                meta,
                scheduled_at: now,
                received_at: now,
            })
        }
    }

    impl TryFrom<EventEntry> for EventEnvelope {
        type Error = EventError;

        fn try_from(entry: EventEntry) -> Result<Self, Self::Error> {
            let metadata = entry
                .meta
                .as_ref()
                .map(|m| serde_json::from_value::<EventMetadata>(m.0.clone()))
                .transpose()
                .map_err(|e| EventError::Conversion(e.to_string()))?;

            let reservation_id = entry
                .reservation_id
                .ok_or_else(|| EventError::MissingReservation { id: entry.id })?;

            Ok(EventEnvelope {
                id: entry.id,
                reservation_id,
                event_type: entry.event_type,
                payload: entry.payload,
                attempts: entry.attempts,
                metadata,
            })
        }
    }

    impl From<EventMetadata> for NewEventMetadata {
        fn from(meta: EventMetadata) -> Self {
            NewEventMetadata {
                event_id: meta.event_id,
                correlation_id: meta.correlation_id,
                causation_id: meta.causation_id,
                source: meta.source,
            }
        }
    }
}

pub(crate) mod schema {
    diesel::table! {
        event_entries (id) {
            id -> Uuid,
            event_type -> Text,
            status -> Int4,
            payload -> Text,
            meta -> Nullable<Jsonb>,
            scheduled_at -> Timestamptz,
            attempts -> Int4,
            reservation_id -> Nullable<Uuid>,
            reserved_at -> Nullable<Timestamptz>,
            received_at -> Timestamptz,
            updated_at -> Timestamptz,
            processed_at -> Nullable<Timestamptz>,
        }
    }
}

pub mod entity {
    use super::models::MetadataJsonb;
    use chrono::{DateTime, Utc};
    use diesel::{Insertable, Queryable, QueryableByName, Selectable};
    use uuid::Uuid;

    #[derive(Debug, Insertable)]
    #[diesel(table_name = super::schema::event_entries)]
    pub struct NewEventEntry {
        pub id: Uuid,
        pub event_type: String,
        pub payload: String,
        pub meta: Option<MetadataJsonb>,
        pub scheduled_at: DateTime<Utc>,
        pub received_at: DateTime<Utc>,
    }

    #[derive(Debug, Queryable, QueryableByName, Selectable)]
    #[diesel(table_name = super::schema::event_entries)]
    pub struct EventEntry {
        pub id: Uuid,
        pub event_type: String,
        pub status: i32,
        pub payload: String,
        pub meta: Option<MetadataJsonb>,
        pub scheduled_at: DateTime<Utc>,
        pub attempts: i32,
        pub reservation_id: Option<Uuid>,
        pub reserved_at: Option<DateTime<Utc>>,
        pub received_at: DateTime<Utc>,
        pub updated_at: DateTime<Utc>,
        pub processed_at: Option<DateTime<Utc>>,
    }
}

mod storage {
    use mulac_diesel::DbPool;

    pub struct EventStoreStorage {
        pub(crate) pool: DbPool,
    }

    impl EventStoreStorage {
        pub fn new(pool: DbPool) -> Self {
            Self { pool }
        }
    }

    pub struct EventConsumerStorage {
        pub(crate) pool: DbPool,
    }

    impl EventConsumerStorage {
        pub(crate) const RETRY_BACKOFF_SECONDS: i64 = 30;
        pub(crate) const MAX_RETRY_BACKOFF_SECONDS: i64 = 120;

        pub fn new(pool: DbPool) -> Self {
            Self { pool }
        }
    }
}

mod store_impl {
    use super::entity::NewEventEntry;
    use super::schema::event_entries;
    use super::storage::EventStoreStorage;
    use crate::assembly::application::io::{
        EventError,
        EventStorePort,
        NewEventEnvelope,
        //
    };
    use diesel::prelude::*;

    impl EventStorePort for EventStoreStorage {
        fn record(&self, envelope: &NewEventEnvelope) -> Result<(), EventError> {
            let mut conn = self
                .pool
                .get()
                .map_err(|e| EventError::Storage(e.to_string()))?;

            let entry = NewEventEntry::try_from(envelope)
                .map_err(|e| EventError::Conversion(e.to_string()))?;

            diesel::insert_into(event_entries::table)
                .values(&entry)
                .on_conflict_do_nothing()
                .execute(&mut conn)
                .map_err(|e| EventError::Storage(e.to_string()))?;

            Ok(())
        }
    }
}

mod consumer_impl {
    use super::entity::EventEntry;
    use super::storage::EventConsumerStorage;
    use crate::assembly::application::io::{
        EventEnvelope,
        EventError,
        EventProcessPort,
        //
    };
    use crate::assembly::domain::{Criterion, EventStatus};
    use crate::event_consumer::io::{EventReservePort, ReservableEventSpec};
    use crate::stale_event_sweep::io::{EventSweepPort, StaleEventSpec};
    use chrono::{DateTime, Duration, Utc};
    use diesel::prelude::*;
    use diesel::sql_types::{Array, BigInt, Int4, Uuid as SqlUuid};
    use uuid::Uuid;

    impl EventReservePort for EventConsumerStorage {
        fn reserve(&self, spec: &ReservableEventSpec) -> Result<Vec<EventEnvelope>, EventError> {
            let mut conn = self
                .pool
                .get()
                .map_err(|e| EventError::Reservation(e.to_string()))?;

            let criteria = ReservationCriteria::from(spec);
            let reservation_id = Uuid::now_v7();

            let entries = diesel::sql_query(
                r#"
                WITH candidates AS (
                    SELECT id
                    FROM event_entries
                    WHERE status = ANY($1)
                      AND scheduled_at <= now()
                      AND attempts < $2
                    ORDER BY scheduled_at ASC
                    LIMIT $3
                    FOR UPDATE SKIP LOCKED
                )
                UPDATE event_entries AS entries
                SET status = $4,
                    reservation_id = $5,
                    reserved_at = now(),
                    attempts = entries.attempts + 1,
                    updated_at = now()
                FROM candidates
                WHERE entries.id = candidates.id
                RETURNING
                    entries.id,
                    entries.event_type,
                    entries.status,
                    entries.payload,
                    entries.meta,
                    entries.scheduled_at,
                    entries.attempts,
                    entries.reservation_id,
                    entries.reserved_at,
                    entries.received_at,
                    entries.updated_at,
                    entries.processed_at
                "#,
            )
            .bind::<Array<Int4>, _>(criteria.statuses)
            .bind::<Int4, _>(criteria.max_attempts)
            .bind::<BigInt, _>(spec.limit as i64)
            .bind::<Int4, _>(i32::from(EventStatus::Reserved))
            .bind::<SqlUuid, _>(reservation_id)
            .load::<EventEntry>(&mut conn)
            .map_err(|e| EventError::Reservation(e.to_string()))?;

            entries.into_iter().map(EventEnvelope::try_from).collect()
        }
    }

    impl EventProcessPort for EventConsumerStorage {
        fn completed(&self, id: Uuid, reservation_id: Uuid) -> Result<(), EventError> {
            use crate::assembly::infra_diesel::schema::event_entries;

            let mut conn = self
                .pool
                .get()
                .map_err(|e| EventError::Storage(e.to_string()))?;

            let updated = diesel::update(
                event_entries::table
                    .filter(event_entries::id.eq(id))
                    .filter(event_entries::reservation_id.eq(reservation_id))
                    .filter(event_entries::status.eq(i32::from(EventStatus::Reserved))),
            )
            .set((
                event_entries::status.eq(i32::from(EventStatus::Completed)),
                event_entries::processed_at.eq(diesel::dsl::now),
                event_entries::updated_at.eq(diesel::dsl::now),
                event_entries::reservation_id.eq(None::<Uuid>),
                event_entries::reserved_at.eq(None::<DateTime<Utc>>),
            ))
            .execute(&mut conn)
            .map_err(|e| EventError::Storage(e.to_string()))?;

            if updated == 0 {
                return Err(EventError::MissingReservation { id });
            }

            Ok(())
        }

        fn failed(
            &self,
            id: Uuid,
            reservation_id: Uuid,
            max_attempts: i32,
        ) -> Result<(), EventError> {
            use crate::assembly::infra_diesel::schema::event_entries;

            let mut conn = self
                .pool
                .get()
                .map_err(|e| EventError::Storage(e.to_string()))?;

            conn.transaction::<(), diesel::result::Error, _>(|conn| {
                let attempts = diesel::update(
                    event_entries::table
                        .filter(event_entries::id.eq(id))
                        .filter(event_entries::reservation_id.eq(reservation_id))
                        .filter(event_entries::status.eq(i32::from(EventStatus::Reserved))),
                )
                .set((
                    event_entries::updated_at.eq(diesel::dsl::now),
                    event_entries::reservation_id.eq(None::<Uuid>),
                    event_entries::reserved_at.eq(None::<DateTime<Utc>>),
                ))
                .returning(event_entries::attempts)
                .get_result::<i32>(conn)?;

                let status = if attempts >= max_attempts {
                    EventStatus::Dead
                } else {
                    EventStatus::Failed
                };

                let retry_delay_seconds =
                    i64::from(attempts.max(1)) * EventConsumerStorage::RETRY_BACKOFF_SECONDS;
                let retry_delay_seconds =
                    retry_delay_seconds.min(EventConsumerStorage::MAX_RETRY_BACKOFF_SECONDS);
                let scheduled_at = Utc::now() + Duration::seconds(retry_delay_seconds);

                diesel::update(event_entries::table.find(id))
                    .set((
                        event_entries::status.eq(i32::from(status)),
                        event_entries::scheduled_at.eq(scheduled_at),
                        event_entries::updated_at.eq(diesel::dsl::now),
                    ))
                    .execute(conn)?;

                Ok(())
            })
            .map_err(|e| match e {
                diesel::result::Error::NotFound => EventError::MissingReservation { id },
                e => EventError::Storage(e.to_string()),
            })?;

            Ok(())
        }
    }

    impl EventSweepPort for EventConsumerStorage {
        fn sweep(&self, spec: &StaleEventSpec) -> Result<u64, EventError> {
            use diesel::sql_types::Timestamptz;

            let mut conn = self
                .pool
                .get()
                .map_err(|e| EventError::Storage(e.to_string()))?;

            let criteria = SweepCriteria::from(spec);
            let now: DateTime<Utc> = Utc::now();

            let affected = diesel::sql_query(
                r#"
                UPDATE event_entries
                SET
                    status         = $1,
                    reservation_id = NULL,
                    reserved_at    = NULL,
                    scheduled_at   = $2 + (attempts * $3 * interval '1 second'),
                    updated_at     = $2
                WHERE status = $4
                  AND reserved_at IS NOT NULL
                  AND reserved_at < $5
                "#,
            )
            .bind::<Int4, _>(i32::from(EventStatus::Failed)) // $1
            .bind::<Timestamptz, _>(now) // $2
            .bind::<BigInt, _>(EventConsumerStorage::RETRY_BACKOFF_SECONDS) // $3
            .bind::<Int4, _>(i32::from(EventStatus::Reserved)) // $4
            .bind::<Timestamptz, _>(criteria.cutoff) // $5
            .execute(&mut conn)
            .map_err(|e| EventError::Storage(e.to_string()))?;

            Ok(affected as u64)
        }
    }

    struct ReservationCriteria {
        statuses: Vec<i32>,
        max_attempts: i32,
    }

    impl From<&ReservableEventSpec> for ReservationCriteria {
        fn from(spec: &ReservableEventSpec) -> Self {
            let mut criteria = Self {
                statuses: vec![],
                max_attempts: spec.max_attempts,
            };

            for criterion in spec.criteria() {
                match criterion {
                    Criterion::StatusIn(statuses) => {
                        criteria.statuses = statuses.into_iter().map(i32::from).collect();
                    }
                    Criterion::MaxAttempts(max) => {
                        criteria.max_attempts = max;
                    }
                    Criterion::ScheduledBeforeNow
                    | Criterion::OrderByScheduledAtAsc
                    | Criterion::ReservedBefore(_) => {}
                }
            }

            criteria
        }
    }

    struct SweepCriteria {
        cutoff: DateTime<Utc>,
    }

    impl From<&StaleEventSpec> for SweepCriteria {
        fn from(spec: &StaleEventSpec) -> Self {
            let mut criteria = Self {
                cutoff: DateTime::<Utc>::MIN_UTC,
            };

            for criterion in spec.criteria() {
                match criterion {
                    Criterion::ReservedBefore(cutoff) => {
                        criteria.cutoff = cutoff;
                    }
                    _ => {}
                }
            }

            criteria
        }
    }
}

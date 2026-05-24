pub mod io {
    pub use mulac_diesel::{DbPool, build_pool};

    pub use super::storage::{InboxConsumerStorage, InboxStoreStorage};
}

mod models {
    use crate::assembly::application::io::InboxMessageMetadata;
    use diesel::deserialize::{FromSql, Result as DeserializeResult};
    use diesel::pg::{Pg, PgValue};
    use diesel::serialize::{IsNull, Output, Result as SerializeResult, ToSql};
    use diesel::sql_types::Jsonb;
    use diesel::{AsExpression, FromSqlRow};
    use serde_json::{from_slice, to_writer};
    use std::io::Write;

    /// Newtype to implement diesel JSONB serialization for `InboxMessageMetadata`
    /// without violating orphan rules (both `ToSql`/`FromSql` and the type are foreign).
    #[derive(Debug, Clone, PartialEq, Eq, AsExpression, FromSqlRow)]
    #[diesel(sql_type = Jsonb)]
    pub struct MetadataJsonb(pub InboxMessageMetadata);

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
    use super::entity::{InboxEntry, NewInboxEntry};
    use super::models::MetadataJsonb;
    use crate::assembly::application::io::{InboxError, InboxMessageEnvelope};
    use crate::assembly::domain::{InboxMessage, InboxStatus};
    use crate::record_messages::io::NewInboxMessageEnvelope;
    use chrono::Utc;

    impl From<NewInboxMessageEnvelope> for NewInboxEntry {
        fn from(message: NewInboxMessageEnvelope) -> Self {
            let now = Utc::now();
            NewInboxEntry::new(
                *message.id(),
                message.payload().to_string(),
                MetadataJsonb(message.meta.into()),
                InboxStatus::Received as i32,
                now,
                0,
                now,
                now,
            )
        }
    }

    impl TryFrom<InboxEntry> for InboxMessageEnvelope {
        type Error = InboxError;

        fn try_from(entry: InboxEntry) -> Result<Self, Self::Error> {
            let status = InboxStatus::try_from(entry.status())
                .map_err(|e| InboxError::Storage(e.to_string()))?;

            Ok(InboxMessageEnvelope {
                msg: InboxMessage {
                    id: entry.id(),
                    payload: entry.payload().to_string(),
                    status,
                    scheduled_at: entry.scheduled_at(),
                    attempts: entry.attempts(),
                    reservation_id: entry.reservation_id(),
                    reserved_at: entry.reserved_at(),
                },
                meta: entry.meta().clone(),
            })
        }
    }
}

pub(crate) mod schema {
    diesel::table! {
        inbox_entries (id) {
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
        }
    }
}

pub(crate) mod entity {
    use super::models::MetadataJsonb;
    use crate::assembly::application::io::InboxMessageMetadata;
    use chrono::{DateTime, Utc};
    use diesel::{Insertable, Queryable, QueryableByName, Selectable};
    use uuid::Uuid;

    #[derive(Debug, Insertable)]
    #[diesel(table_name = super::schema::inbox_entries)]
    pub struct NewInboxEntry {
        id: Uuid,
        payload: String,
        meta: MetadataJsonb,
        status: i32,
        scheduled_at: DateTime<Utc>,
        attempts: i32,
        received_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    }

    impl NewInboxEntry {
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
                payload,
                meta,
                status,
                scheduled_at,
                attempts,
                received_at,
                updated_at,
            }
        }
    }

    #[derive(Debug, Queryable, QueryableByName, Selectable)]
    #[diesel(table_name = super::schema::inbox_entries)]
    pub struct InboxEntry {
        id: Uuid,
        payload: String,
        meta: MetadataJsonb,
        status: i32,
        scheduled_at: DateTime<Utc>,
        attempts: i32,
        reservation_id: Option<Uuid>,
        reserved_at: Option<DateTime<Utc>>,
        received_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
        processed_at: Option<DateTime<Utc>>,
    }

    impl InboxEntry {
        pub fn id(&self) -> Uuid {
            self.id
        }
        pub fn payload(&self) -> &str {
            &self.payload
        }
        pub fn meta(&self) -> &InboxMessageMetadata {
            &self.meta.0
        }
        pub fn status(&self) -> i32 {
            self.status
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
    }
}

mod storage {
    use mulac_diesel::DbPool;

    pub struct InboxStoreStorage {
        pub(crate) pool: DbPool,
    }

    impl InboxStoreStorage {
        pub fn new(pool: DbPool) -> Self {
            Self { pool }
        }
    }

    pub struct InboxConsumerStorage {
        pub(crate) pool: DbPool,
    }

    impl InboxConsumerStorage {
        pub(crate) const RETRY_BACKOFF_SECONDS: i64 = 30;

        pub fn new(pool: DbPool) -> Self {
            Self { pool }
        }
    }
}

pub mod io {
    pub use super::entity::{CommandEntry, NewCommandEntry};
    pub use super::storage::{CommandConsumerStorage, CommandStoreStorage};
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
    use super::entity::{CommandEntry, NewCommandEntry};
    use super::models::MetadataJsonb;
    use crate::assembly::application::io::{
        Command,
        CommandEnvelope,
        CommandError,
        CommandMetadata,
        NewCommandEnvelope,
        NewCommandMetadata,
        //
    };
    use chrono::Utc;

    impl TryFrom<&NewCommandEnvelope> for NewCommandEntry {
        type Error = CommandError;

        fn try_from(envelope: &NewCommandEnvelope) -> Result<Self, Self::Error> {
            let metadata = envelope.metadata.as_ref().ok_or_else(|| {
                CommandError::Conversion("command_id is required: metadata is missing".into())
            })?;

            let now = Utc::now();
            let meta = serde_json::to_value(metadata).ok().map(MetadataJsonb);

            Ok(NewCommandEntry {
                id: metadata.command_id,
                command_type: envelope.command.command_type.clone(),
                payload: envelope.command.payload.clone(),
                meta,
                scheduled_at: now,
                received_at: now,
            })
        }
    }

    impl TryFrom<CommandEntry> for CommandEnvelope {
        type Error = CommandError;

        fn try_from(entry: CommandEntry) -> Result<Self, Self::Error> {
            let metadata = entry
                .meta
                .as_ref()
                .map(|m| serde_json::from_value::<CommandMetadata>(m.0.clone()))
                .transpose()
                .map_err(|e| CommandError::Conversion(e.to_string()))?;

            let reservation_id = entry
                .reservation_id
                .ok_or_else(|| CommandError::MissingReservation { id: entry.id })?;

            Ok(CommandEnvelope {
                command: Command {
                    id: entry.id,
                    reservation_id,
                    command_type: entry.command_type,
                    payload: entry.payload,
                    attempts: entry.attempts,
                },
                metadata,
            })
        }
    }

    impl From<CommandMetadata> for NewCommandMetadata {
        fn from(meta: CommandMetadata) -> Self {
            NewCommandMetadata {
                command_id: meta.command_id,
                correlation_id: meta.correlation_id,
                causation_id: meta.causation_id,
                source: meta.source,
            }
        }
    }
}

pub(crate) mod schema {
    diesel::table! {
        command_entries (id) {
            id -> Uuid,
            command_type -> Text,
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
    #[diesel(table_name = super::schema::command_entries)]
    pub struct NewCommandEntry {
        pub id: Uuid,
        pub command_type: String,
        pub payload: String,
        pub meta: Option<MetadataJsonb>,
        pub scheduled_at: DateTime<Utc>,
        pub received_at: DateTime<Utc>,
    }

    #[derive(Debug, Queryable, QueryableByName, Selectable)]
    #[diesel(table_name = super::schema::command_entries)]
    pub struct CommandEntry {
        pub id: Uuid,
        pub command_type: String,
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

    pub struct CommandStoreStorage {
        pub(crate) pool: DbPool,
    }

    impl CommandStoreStorage {
        pub fn new(pool: DbPool) -> Self {
            Self { pool }
        }
    }

    pub struct CommandConsumerStorage {
        pub(crate) pool: DbPool,
    }

    impl CommandConsumerStorage {
        pub(crate) const RETRY_BACKOFF_SECONDS: i64 = 30;
        pub(crate) const MAX_RETRY_BACKOFF_SECONDS: i64 = 120;

        pub fn new(pool: DbPool) -> Self {
            Self { pool }
        }
    }
}

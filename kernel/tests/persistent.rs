//! Live tests use isolated schemas and uniquely named broker resources.
//! KERNEL_TEST_DATABASE_URL and KERNEL_TEST_AMQP_URL opt in via --ignored.
mod support;
use diesel::{
    connection::SimpleConnection,
    prelude::*,
    sql_types::{BigInt, Jsonb, Text, Uuid as SqlUuid},
};
use mulac_kernel::io::*;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use support::*;
use uuid::Uuid;

struct Database {
    admin: DbPool,
    pool: DbPool,
    schema: String,
    url: String,
}
impl Database {
    fn new() -> Self {
        let url = std::env::var("KERNEL_TEST_DATABASE_URL").expect("set KERNEL_TEST_DATABASE_URL");
        let admin = build_pool(&url).unwrap();
        let schema = format!("kernel_qa01_{}", Uuid::now_v7().simple());
        admin
            .get()
            .unwrap()
            .batch_execute(&format!("CREATE SCHEMA {schema}"))
            .unwrap();
        let separator = if url.contains('?') { '&' } else { '?' };
        let url = format!("{url}{separator}options=-csearch_path%3D{schema}");
        let pool = build_pool(&url).unwrap();
        let mut connection = pool.get().unwrap();
        connection
            .batch_execute(include_str!(
                "../../test_apps/twitter/migrations/2025-01-01-000001_infrastructure/up.sql"
            ))
            .unwrap();
        connection
            .batch_execute(include_str!(
                "../../test_apps/twitter/migrations/2026-05-27-000001_entry_extra_info/up.sql"
            ))
            .unwrap();
        Self {
            admin,
            pool,
            schema,
            url,
        }
    }
    fn count(&self, table: &str) -> i64 {
        #[derive(QueryableByName)]
        struct Count {
            #[diesel(sql_type = BigInt)]
            count: i64,
        }
        diesel::sql_query(format!("SELECT COUNT(*) AS count FROM {table}"))
            .get_result::<Count>(&mut self.pool.get().unwrap())
            .unwrap()
            .count
    }
}
impl Drop for Database {
    fn drop(&mut self) {
        self.admin
            .get()
            .unwrap()
            .batch_execute(&format!("DROP SCHEMA {} CASCADE", self.schema))
            .unwrap();
    }
}

async fn eventually(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("work completed before deadline");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires KERNEL_TEST_DATABASE_URL"]
async fn persistent_registrations_run_and_inbox_acceptance_is_durable() {
    let db = Database::new();
    let received = Arc::new(Mutex::new(vec![]));
    let calls = Arc::new(AtomicUsize::new(0));
    let factories = Arc::new(AtomicUsize::new(0));
    let factory_count = factories.clone();
    // Explicit pool must take precedence over an unusable configured database URL.
    let kernel = boot(KernelConfig {
        database_url: Some("unused".into()),
        ..Default::default()
    })
    .command_handler("start", Arc::new(StartHandler))
    .command_handler("follow", Arc::new(FollowHandler(calls.clone())))
    .event_subscriber("started", "capture", Arc::new(Capture(received.clone())))
    .event_subscriber_with_command_gateway("started", "follow", move |gateway| {
        factory_count.fetch_add(1, Ordering::SeqCst);
        Arc::new(FollowSubscriber(gateway))
    })
    .start_persistent(db.pool.clone(), 2)
    .unwrap();
    kernel.state().dispatch_command(envelope(Start)).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(factories.load(Ordering::SeqCst), 1);
    assert_eq!(received.lock().unwrap().len(), 1);
    assert_eq!(db.count("command_entries WHERE status = 5"), 2);
    assert_eq!(db.count("event_entries WHERE status = 5"), 1);
    let id = Uuid::now_v7();
    let recorder = kernel.state().inbox_recorder;
    recorder
        .publish(NewInboxMessageEnvelope::from(InboundMessageEnvelope {
            message_id: Some(id),
            payload: "durable payload".into(),
            correlation_id: None,
            source: Some("test".into()),
            routing_key: Some("start".into()),
        }))
        .unwrap();
    assert_eq!(db.count("inbox_entries"), 1);
    kernel.shutdown();
    kernel.wait().await.unwrap();
    // Verify through a fresh pool after the kernel has terminated.
    let fresh_pool = build_pool(&db.url).unwrap();
    #[derive(QueryableByName)]
    struct Payload {
        #[diesel(sql_type = Text)]
        payload: String,
    }
    let stored: Payload = diesel::sql_query("SELECT payload FROM inbox_entries WHERE id = $1")
        .bind::<SqlUuid, _>(id)
        .get_result(&mut fresh_pool.get().unwrap())
        .unwrap();
    assert_eq!(stored.payload, "durable payload");
    db.pool
        .get()
        .unwrap()
        .batch_execute("DROP TABLE inbox_entries")
        .unwrap();
    assert!(matches!(
        recorder.publish(NewInboxMessageEnvelope::from(InboundMessageEnvelope {
            message_id: Some(Uuid::now_v7()),
            payload: "not accepted".into(),
            correlation_id: None,
            source: None,
            routing_key: None,
        })),
        Err(InboxError::Storage(_))
    ));
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires KERNEL_TEST_DATABASE_URL"]
async fn configured_database_and_background_workers_execute_follow_ups_without_manual_draining() {
    let db = Database::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let kernel = boot(KernelConfig {
        database_url: Some(db.url.clone()),
        ..Default::default()
    })
    .command_handler("start", Arc::new(StartHandler))
    .command_handler("follow", Arc::new(FollowHandler(calls.clone())))
    .event_subscriber_with_command_gateway("started", "follow", |gateway| {
        Arc::new(FollowSubscriber(gateway))
    })
    .start_persistent(None, 0)
    .unwrap();
    kernel.state().dispatch_command(envelope(Start)).unwrap();
    eventually(|| calls.load(Ordering::SeqCst) == 1).await;
    assert_eq!(db.count("event_entries WHERE status = 5"), 1);
    kernel.shutdown();
    kernel.wait().await.unwrap();
}

#[derive(QueryableByName)]
struct Outbound {
    #[diesel(sql_type = Text)]
    payload: String,
    #[diesel(sql_type = Jsonb)]
    meta: serde_json::Value,
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires KERNEL_TEST_DATABASE_URL and KERNEL_TEST_AMQP_URL"]
async fn configured_outbox_records_and_publishes_and_inbox_intake_stores() {
    use lapin::{
        BasicProperties, Connection, ConnectionProperties, ExchangeKind, options::*,
        types::FieldTable,
    };
    let db = Database::new();
    let url = std::env::var("KERNEL_TEST_AMQP_URL").expect("set KERNEL_TEST_AMQP_URL");
    let connection = Connection::connect(&url, ConnectionProperties::default())
        .await
        .unwrap();
    let channel = connection.create_channel().await.unwrap();
    let exchange = format!("kernel.qa01.{}", Uuid::now_v7().simple());
    let inbound_exchange = format!("{exchange}.inbox");
    let queue = format!("{exchange}.outbound");
    let inbound_queue = format!("{exchange}.inbound");
    for exchange in [&exchange, &inbound_exchange] {
        channel
            .exchange_declare(
                exchange.clone().into(),
                ExchangeKind::Topic,
                ExchangeDeclareOptions::default(),
                FieldTable::default(),
            )
            .await
            .unwrap();
    }
    channel
        .queue_declare(
            queue.clone().into(),
            QueueDeclareOptions::default(),
            FieldTable::default(),
        )
        .await
        .unwrap();
    channel
        .queue_bind(
            queue.clone().into(),
            exchange.clone().into(),
            "qa01.#".into(),
            QueueBindOptions::default(),
            FieldTable::default(),
        )
        .await
        .unwrap();
    let kernel = boot(KernelConfig {
        amqp_url: Some(url),
        inbox_queue: Some(inbound_queue.clone()),
        inbox_exchange: Some(inbound_exchange.clone()),
        outbox_exchange: Some(exchange.clone()),
        outbox_content_type: Some("application/qa01+json".into()),
        ..Default::default()
    })
    .command_handler("start", Arc::new(StartHandler))
    .outbox_subscriber("started")
    .start_persistent(db.pool.clone(), 0)
    .unwrap();
    let sent = event("started");
    let metadata = sent.metadata.as_ref().unwrap();
    let id = metadata.event_id;
    kernel
        .state()
        .event_gateway()
        .dispatch(sent.clone())
        .unwrap();
    kernel
        .event_consumer()
        .consume(&ReservableEventSpec::new(10))
        .unwrap();
    let stored: Outbound =
        diesel::sql_query("SELECT payload, meta FROM outbox_entries WHERE id = $1")
            .bind::<SqlUuid, _>(id)
            .get_result(&mut db.pool.get().unwrap())
            .unwrap();
    assert_eq!(stored.payload, sent.payload);
    assert_eq!(stored.meta["event_id"], id.to_string());
    assert_eq!(stored.meta["routing_key"], "qa01.started");
    assert_eq!(stored.meta["content_type"], "application/qa01+json");
    assert_eq!(
        stored.meta["correlation_id"],
        metadata.correlation_id.unwrap().to_string()
    );
    assert_eq!(db.count("event_entries WHERE status = 5"), 1);
    let published = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(delivery) = channel
                .basic_get(queue.clone().into(), BasicGetOptions { no_ack: true })
                .await
                .unwrap()
            {
                break delivery;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(published.data, sent.payload.as_bytes());
    assert_eq!(published.routing_key.as_str(), "qa01.started");
    assert_eq!(published.exchange.as_str(), exchange);
    assert_eq!(
        published
            .properties
            .content_type()
            .as_ref()
            .unwrap()
            .as_str(),
        "application/qa01+json"
    );
    assert_eq!(
        published.properties.message_id().as_ref().unwrap().as_str(),
        id.to_string()
    );
    let inbound_id = Uuid::now_v7();
    channel
        .basic_publish(
            inbound_exchange.clone().into(),
            "commands.start".into(),
            BasicPublishOptions::default(),
            b"inbound work",
            BasicProperties::default().with_message_id(inbound_id.to_string().into()),
        )
        .await
        .unwrap()
        .await
        .unwrap();
    eventually(|| db.count("inbox_entries") == 1).await;
    let inbound: Outbound =
        diesel::sql_query("SELECT payload, meta FROM inbox_entries WHERE id = $1")
            .bind::<SqlUuid, _>(inbound_id)
            .get_result(&mut db.pool.get().unwrap())
            .unwrap();
    assert_eq!(inbound.payload, "inbound work");
    assert_eq!(inbound.meta["routing_key"], "commands.start");
    // Missing routing is not a completed event and cannot create outbound work.
    let mut invalid = event("started");
    invalid.metadata.as_mut().unwrap().routing_key = None;
    kernel.state().event_gateway().dispatch(invalid).unwrap();
    assert!(
        kernel
            .event_consumer()
            .consume(&ReservableEventSpec::new(10))
            .is_err()
    );
    assert_eq!(db.count("event_entries WHERE status = 4"), 1);
    assert_eq!(db.count("outbox_entries"), 1);
    // Application events can opt into outbound routing without bypassing command handling.
    kernel.state().dispatch_command(envelope(Start)).unwrap();
    kernel
        .command_consumer()
        .consume(&ReservableCommandSpec::new(10))
        .unwrap();
    kernel
        .event_consumer()
        .consume(&ReservableEventSpec::new(10))
        .unwrap();
    assert_eq!(db.count("outbox_entries"), 2);
    kernel.shutdown();
    kernel.wait().await.unwrap();
    for queue in [&queue, &inbound_queue] {
        channel
            .queue_delete(queue.clone().into(), QueueDeleteOptions::default())
            .await
            .unwrap();
    }
    for exchange in [&exchange, &inbound_exchange] {
        channel
            .exchange_delete(exchange.clone().into(), ExchangeDeleteOptions::default())
            .await
            .unwrap();
    }
}

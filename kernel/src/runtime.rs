//! Persistent-mode infrastructure and owned background work.
use crate::io::{KernelConfig, KernelError, block_on_blocking};
use commanding::io::{CommandConsumer, CommandConsumerStorage, CommandSweeper, StaleCommandSpec};
use eventing::io::{EventConsumer, EventConsumerStorage, EventSweeper, StaleEventSpec};
use inbox::io::InboxRecorder;
use mulac_diesel::DbPool;
use std::{sync::Arc, time::Duration};
use tokio::{
    runtime::{Handle, RuntimeFlavor},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

pub(crate) fn require_runtime() -> Result<(), KernelError> {
    match Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == RuntimeFlavor::MultiThread => Ok(()),
        _ => Err(KernelError::Unsupported(
            "persistent startup requires a live multi-thread Tokio runtime".into(),
        )),
    }
}

pub(crate) struct PersistentRuntime {
    token: CancellationToken,
    joins: Vec<JoinHandle<Result<(), KernelError>>>,
    // Retain the connection until all workers have stopped.
    _connection: Option<lapin::Connection>,
}

impl PersistentRuntime {
    pub(crate) fn start(
        config: &KernelConfig,
        pool: DbPool,
        inbox_recorder: Arc<InboxRecorder>,
        command_consumer: Arc<CommandConsumer>,
        event_consumer: Arc<EventConsumer>,
        outbox: bool,
        token: CancellationToken,
    ) -> Result<Self, KernelError> {
        let config = config.clone();
        let transport_pool = pool.clone();
        // Connect and set up transports before spawning any worker. Startup
        // failures cannot leave detached workers processing application data.
        let (connection, inbox_worker, outbox_consumer) = block_on_blocking(async move {
            let Some(url) = config.amqp_url.as_deref() else {
                return Ok((None, None, None));
            };
            let connection = inbox::io::connection(url)
                .await
                .map_err(|error| KernelError::Amqp(error.to_string()))?;
            let inbox_worker = if let Some(queue) = &config.inbox_queue {
                let channel = connection.create_channel().await.map_err(amqp_error)?;
                let worker = inbox::io::AmqpWorker::new(&channel, queue, (*inbox_recorder).clone())
                    .await
                    .map_err(|error| KernelError::Amqp(error.to_string()))?;
                // The exchange is application-provisioned; bind all its routing keys.
                channel
                    .queue_bind(
                        queue.clone().into(),
                        config.inbox_exchange.as_ref().unwrap().clone().into(),
                        "#".into(),
                        lapin::options::QueueBindOptions::default(),
                        lapin::types::FieldTable::default(),
                    )
                    .await
                    .map_err(amqp_error)?;
                Some(worker)
            } else {
                None
            };
            let outbox_consumer = if outbox {
                let channel = connection.create_channel().await.map_err(amqp_error)?;
                let publisher = outbox::io::AmqpPublisher::new(
                    channel,
                    outbox::io::AmqpPublishConfig {
                        exchange: config.outbox_exchange.unwrap(),
                        default_content_type: config
                            .outbox_content_type
                            .unwrap_or_else(|| "application/json".into()),
                        mandatory: true,
                    },
                );
                Some(Arc::new(outbox::io::OutboxConsumer::new(
                    outbox::io::consumer_repository(transport_pool),
                    Arc::new(publisher),
                )))
            } else {
                None
            };
            Ok::<_, KernelError>((Some(connection), inbox_worker, outbox_consumer))
        })?;

        let mut joins = vec![
            poll(token.clone(), Duration::from_secs(1), move || {
                command_consumer
                    .consume(&commanding::io::ReservableCommandSpec::new(10))
                    .map(|_| ())
                    .map_err(crate::io::first_command_error)
            }),
            poll(token.clone(), Duration::from_secs(1), move || {
                event_consumer
                    .consume(&eventing::io::ReservableEventSpec::new(10))
                    .map(|_| ())
                    .map_err(crate::io::first_event_error)
            }),
        ];
        if let Some(mut worker) = inbox_worker {
            let token = token.clone();
            joins.push(tokio::spawn(async move {
                worker.run(token).await.map_err(KernelError::from)
            }));
        }
        if let Some(consumer) = outbox_consumer {
            joins.push(poll(token.clone(), Duration::from_secs(1), move || {
                consumer
                    .publish_batch(&outbox::io::ReservableOutboxSpec::new(10))
                    .map(|_| ())
                    .map_err(|errors| KernelError::Worker(format!("outbox consumer: {errors:?}")))
            }));
        }
        let commands = CommandSweeper::new(Arc::new(CommandConsumerStorage::new(pool.clone())));
        let events = EventSweeper::new(Arc::new(EventConsumerStorage::new(pool.clone())));
        let inbox = inbox::io::ReservationSweeper::new(Arc::new(
            inbox::io::InboxConsumerStorage::new(pool.clone()),
        ));
        let outbox_sweeper = outbox::io::reservation_sweeper(pool);
        joins.push(poll(token.clone(), Duration::from_secs(60), move || {
            let timeout = chrono::Duration::minutes(5);
            // Attempt every sweep even when one adapter fails.
            let results = [
                commands
                    .sweep(&StaleCommandSpec::new(timeout))
                    .map(|_| ())
                    .map_err(KernelError::from),
                events
                    .sweep(&StaleEventSpec::new(timeout))
                    .map(|_| ())
                    .map_err(KernelError::from),
                inbox
                    .sweep(&inbox::io::StaleReservationSpec::new(timeout))
                    .map(|_| ())
                    .map_err(KernelError::from),
                if outbox {
                    outbox_sweeper
                        .sweep(&outbox::io::StaleReservationSpec::new(timeout))
                        .map(|_| ())
                        .map_err(KernelError::from)
                } else {
                    Ok(())
                },
            ];
            let mut failure = None;
            for result in results {
                if let Err(error) = result {
                    tracing::error!("reservation sweeper: {error}");
                    failure = Some(error);
                }
            }
            failure.map_or(Ok(()), Err)
        }));
        Ok(Self {
            token,
            joins,
            _connection: connection,
        })
    }

    pub(crate) async fn wait(mut self) -> Result<(), KernelError> {
        let mut failure = None;
        for join in self.joins.drain(..) {
            let result = join
                .await
                .unwrap_or_else(|error| Err(KernelError::Worker(error.to_string())));
            if let Err(error) = result {
                failure.get_or_insert(error);
            }
        }
        failure.map_or(Ok(()), Err)
    }
}

impl Drop for PersistentRuntime {
    fn drop(&mut self) {
        self.token.cancel();
    }
}

fn amqp_error(error: lapin::Error) -> KernelError {
    KernelError::Amqp(error.to_string())
}

fn poll<F>(
    token: CancellationToken,
    interval: Duration,
    work: F,
) -> JoinHandle<Result<(), KernelError>>
where
    F: Fn() -> Result<(), KernelError> + Send + Sync + 'static,
{
    let work = Arc::new(work);
    tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                _ = token.cancelled() => return Ok(()),
                _ = tokio::time::sleep(interval) => {}
            }
            if token.is_cancelled() {
                return Ok(());
            }
            let work = work.clone();
            // Per-entry/storage errors remain retriable; a panic is a worker failure.
            if let Err(error) = tokio::task::spawn_blocking(move || work())
                .await
                .map_err(|error| KernelError::Worker(error.to_string()))?
            {
                tracing::error!("persistent worker: {error}");
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn waiting_observes_workers_without_initiating_cancellation() {
        let token = CancellationToken::new();
        let worker_token = token.clone();
        let runtime = PersistentRuntime {
            token: token.clone(),
            joins: vec![tokio::spawn(async move {
                worker_token.cancelled().await;
                Ok(())
            })],
            _connection: None,
        };
        let mut waiting = Box::pin(runtime.wait());
        assert!(
            tokio::time::timeout(Duration::from_millis(10), waiting.as_mut())
                .await
                .is_err()
        );
        assert!(!token.is_cancelled());
        token.cancel();
        tokio::time::timeout(Duration::from_secs(1), waiting)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn waiting_retains_worker_errors_and_panics_until_siblings_stop() {
        for panic in [false, true] {
            let token = CancellationToken::new();
            let sibling_token = token.clone();
            let runtime = PersistentRuntime {
                token: token.clone(),
                joins: vec![
                    tokio::spawn(async move {
                        assert!(!panic, "worker panic");
                        Err(KernelError::Worker("worker failure".into()))
                    }),
                    tokio::spawn(async move {
                        sibling_token.cancelled().await;
                        Ok(())
                    }),
                ],
                _connection: None,
            };
            let mut waiting = Box::pin(runtime.wait());
            assert!(
                tokio::time::timeout(Duration::from_millis(10), waiting.as_mut())
                    .await
                    .is_err()
            );
            assert!(
                !token.is_cancelled(),
                "worker failure must not cancel its sibling"
            );
            token.cancel();
            let result = tokio::time::timeout(Duration::from_secs(1), waiting)
                .await
                .unwrap();
            assert!(matches!(result, Err(KernelError::Worker(message)) if
                message.contains(if panic { "worker panic" } else { "worker failure" })));
        }
    }
}

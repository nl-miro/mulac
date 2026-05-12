use inbox::io::{AmqpWorker, InboxRecorder, connection, repository};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

pub struct Config {
    pub database_url: String,
    pub amqp_url: String,
    pub queue: String,
    pub worker_count: usize,
}

pub async fn run(config: Config, token: CancellationToken) -> anyhow::Result<()> {
    //let conn = Connection::connect(&config.amqp_url, ConnectionProperties::default()).await?;
    let conn = connection(&config.amqp_url).await?;

    let repository = repository(config.database_url)
        .map_err(|e| anyhow::anyhow!("failed to initialize repository: {e}"))?;

    let mut tasks = JoinSet::new();

    for i in 0..config.worker_count {
        let recorder = InboxRecorder::new(repository.clone());
        let channel = conn.create_channel().await?;
        let token = token.clone();
        let queue = config.queue.clone();

        tasks.spawn(async move {
            let consumer_tag = format!("test_app_worker_{i}");
            let mut worker =
                AmqpWorker::new_with_consumer_tag(&channel, &queue, consumer_tag, recorder)
                    .await
                    .map_err(|e| anyhow::anyhow!("worker {i} failed to start: {e}"))?;

            worker
                .run(token)
                .await
                .map_err(|e| anyhow::anyhow!("worker {i} stopped: {e}"))
        });
    }

    while let Some(result) = tasks.join_next().await {
        result??;
    }

    Ok(())
}

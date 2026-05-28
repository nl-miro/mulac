use anyhow::Result;
use kernel::io::{
    CommandDispatcher, CommandGateway, CommandHandlerRegistry, EventSubscriberRegistry,
};
use kernel::{
    CommandHandlers, EventDispatcher, EventGateway, InboxRecorder, InboxRecorderRepository,
    NoopInboxStore,
};
use poem::{
    EndpointExt,
    Route,
    get,
    handler,
    listener::TcpListener,
    middleware::AddData, //
};
use poem_openapi::OpenApiService;
use std::env;
use std::sync::Arc;
use test_app_twitter::io::{
    AppState,
    DEFAULT_DATABASE_URL,
    DirectMessageSendApi,
    FollowUserApi,
    InboxApi,
    OutboxApi,
    TweetDeleteApi,
    TweetLikeApi,
    TweetPostApi,
    TweetRetweetApi,
    TweetUnlikeApi,
    UnfollowUserApi,
    run_command_worker,
    run_event_worker,
    start_mulac, //
};
use test_app_twitter::io::{build_pool, run_migrations};
use tracing_subscriber::EnvFilter;

#[handler]
fn health() -> &'static str {
    "ok"
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let command = env::args().nth(1).unwrap_or_else(|| "serve".to_string());
    let database_url =
        env::var("DATABASE_URL").unwrap_or_else(|_| DEFAULT_DATABASE_URL.to_string());

    match command.as_str() {
        "migrate" => {
            let pool = build_pool(&database_url)?;
            run_migrations(&pool)?;
            println!("migrations applied");
        }

        "serve" => {
            let bind_addr = env::var("BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:33002".to_string());

            let pool = build_pool(&database_url)?;
            run_migrations(&pool)?;

            let kernel = start_mulac(pool.clone())
                .map_err(|e| anyhow::anyhow!("start_mulac failed: {e}"))?;
            let token = kernel.child_token();
            tokio::spawn(run_command_worker(kernel.command_consumer(), token.clone()));
            tokio::spawn(run_event_worker(kernel.event_consumer(), token));

            let subscribers = vec![];

            let event_registry = Arc::new(EventSubscriberRegistry::from_subscribers(subscribers));
            let event_dispatcher = Arc::new(EventDispatcher::new(event_registry));
            let event_gateway = Arc::new(EventGateway::direct(event_dispatcher.clone()));

            let command_handlers = CommandHandlers::new();

            let command_registry =
                Arc::new(CommandHandlerRegistry::from_handlers(command_handlers));
            let command_dispatcher = Arc::new(CommandDispatcher::new(
                command_registry,
                event_gateway.clone(),
            ));
            let command_gateway = Arc::new(CommandGateway::direct(command_dispatcher.clone()));

            let inbox_recorder = Arc::new(InboxRecorder::new(Arc::new(
                InboxRecorderRepository::new(Arc::new(NoopInboxStore)),
            )));

            let _state = kernel::AppState {
                inbox_recorder,
                command_gateway,
                event_gateway,
            };

            let state = AppState::new(pool, kernel.state());

            let api = OpenApiService::new(
                (
                    TweetPostApi,
                    TweetDeleteApi,
                    TweetRetweetApi,
                    FollowUserApi,
                    UnfollowUserApi,
                    TweetLikeApi,
                    TweetUnlikeApi,
                    DirectMessageSendApi,
                    InboxApi,
                    OutboxApi,
                ),
                "Twitter Test App",
                "0.1.0",
            )
            .server(format!("http://{bind_addr}/api"));

            let swagger = api.swagger_ui();
            let app = Route::new()
                .at("/health", get(health))
                .nest("/api", api)
                .nest("/swagger", swagger)
                .with(AddData::new(state));

            tracing::info!(%bind_addr, "starting test_app_twitter");

            tokio::select! {
                result = poem::Server::new(TcpListener::bind(bind_addr)).run(app) => result?,
                _ = tokio::signal::ctrl_c() => kernel.shutdown(),
            }
            kernel.wait().await?;
        }

        other => anyhow::bail!("unknown command `{other}`; expected `serve` or `migrate`"),
    }

    Ok(())
}

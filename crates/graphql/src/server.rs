//! GraphQL HTTP server.

use std::future::Future;

use async_graphql::http::GraphiQLSource;
use async_graphql::{EmptyMutation, EmptySubscription, ObjectType, Schema};
use async_graphql_axum::{GraphQLRequest, GraphQLResponse};
use axum::{
    Router,
    extract::State,
    response::{Html, IntoResponse},
    routing::get,
};
use tracing::{debug, info};

/// Server configuration.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub enable_playground: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 4000,
            enable_playground: true,
        }
    }
}

/// Start the GraphQL server with any schema type.
pub async fn serve<Q>(
    schema: Schema<Q, EmptyMutation, EmptySubscription>,
    config: ServerConfig,
) -> Result<(), std::io::Error>
where
    Q: ObjectType + 'static,
{
    let mut app = Router::new()
        .route("/graphql", get(graphql_playground).post(graphql_handler::<Q>))
        .route("/health", get(health_check))
        .with_state(schema);

    if config.enable_playground {
        app = app.route("/", get(graphql_playground));
    }

    let addr = format!("{}:{}", config.host, config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    info!("⚡ GraphQL server listening on http://{}", addr);

    axum::serve(listener, app).await
}

/// Start the GraphQL server with graceful shutdown support.
pub async fn serve_with_shutdown<Q, F>(
    schema: Schema<Q, EmptyMutation, EmptySubscription>,
    config: ServerConfig,
    shutdown_signal: F,
) -> Result<(), std::io::Error>
where
    Q: ObjectType + 'static,
    F: Future<Output = ()> + Send + 'static,
{
    let mut app = Router::new()
        .route("/graphql", get(graphql_playground).post(graphql_handler::<Q>))
        .route("/health", get(health_check))
        .with_state(schema);

    if config.enable_playground {
        app = app.route("/", get(graphql_playground));
    }

    let addr = format!("{}:{}", config.host, config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    debug!(addr = %addr, "Server listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal)
        .await
}

/// GraphQL query handler.
async fn graphql_handler<Q>(
    State(schema): State<Schema<Q, EmptyMutation, EmptySubscription>>,
    req: GraphQLRequest,
) -> GraphQLResponse
where
    Q: ObjectType + 'static,
{
    schema.execute(req.into_inner()).await.into()
}

/// GraphQL Playground UI.
async fn graphql_playground() -> impl IntoResponse {
    Html(GraphiQLSource::build().endpoint("/graphql").finish())
}

/// Health check endpoint.
async fn health_check() -> &'static str {
    "OK"
}

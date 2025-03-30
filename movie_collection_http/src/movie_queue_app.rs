#![allow(clippy::needless_pass_by_value)]

use async_graphql::{
    dataloader::DataLoader,
    http::{playground_source, GraphQLPlaygroundConfig, GraphiQLSource},
    EmptyMutation, EmptySubscription, Schema,
};
use async_graphql_axum::GraphQL;
use axum::http::{Method, StatusCode};
use log::debug;
use stack_string::format_sstr;
use std::{convert::TryInto, net::SocketAddr, time::Duration};
use tokio::{
    fs::{create_dir, remove_dir_all},
    net::TcpListener,
    time::interval,
};
use tower_http::cors::{Any, CorsLayer};
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;

use movie_collection_lib::{config::Config, pgpool::PgPool, trakt_connection::TraktConnection};

use super::{
    errors::ServiceError as Error,
    graphql::{ItemLoader, QueryRoot},
    logged_user::{fill_from_db, get_secrets},
    movie_queue_routes::{get_full_path, ApiDoc},
};

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub db: PgPool,
    pub trakt: TraktConnection,
}

/// # Errors
/// Return error if app fails to start
pub async fn start_app() -> Result<(), Error> {
    async fn update_db(pool: PgPool) {
        let mut i = interval(Duration::from_secs(60));
        loop {
            fill_from_db(&pool).await.unwrap_or(());
            i.tick().await;
        }
    }
    let config = Config::with_config()?;
    get_secrets(&config.secret_path, &config.jwt_secret_path).await?;

    if let Some(partial_path) = &config.video_playback_path {
        let partial_path = partial_path.join("videos").join("partial");
        if partial_path.exists() {
            remove_dir_all(&partial_path).await?;
            create_dir(&partial_path).await?;
        }
    }

    let pool = PgPool::new(&config.pgurl)?;
    let trakt = TraktConnection::new(config.clone());

    tokio::task::spawn(update_db(pool.clone()));

    let port = config.port;

    run_app(config, pool, trakt, port).await
}

async fn run_app(
    config: Config,
    pool: PgPool,
    trakt: TraktConnection,
    port: u32,
) -> Result<(), Error> {
    let schema = Schema::build(QueryRoot, EmptyMutation, EmptySubscription)
        .data(DataLoader::new(
            ItemLoader::new(pool.clone()),
            tokio::task::spawn,
        ))
        .finish();

    // let graphql_post = rweb::path!("list" / "graphql" / "graphql")
    //     .and(async_graphql_warp::graphql(schema))
    //     .and_then(
    //         |(schema, request): (
    //             Schema<QueryRoot, EmptyMutation, EmptySubscription>,
    //             async_graphql::Request,
    //         )| async move {
    //             Ok::<_,
    // Infallible>(GraphQLResponse::from(schema.execute(request).await))
    //         },
    //     );
    // let graphql_playground = rweb::path!("list" / "graphql" / "playground")
    //     .and(rweb::path::end())
    //     .and(rweb::get())
    //     .map(|| {
    //         HttpResponse::builder()
    //             .header("content-type", "text/html")
    //             .body(playground_source(GraphQLPlaygroundConfig::new("/")))
    //     })
    //     .boxed();

    let app = AppState {
        config,
        db: pool,
        trakt,
    };

    let cors = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST])
        .allow_headers(["content-type".try_into()?, "jwt".try_into()?])
        .allow_origin(Any);

    let (router, api) = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .merge(get_full_path(&app))
        .layer(cors)
        .split_for_parts();

    let spec_json = serde_json::to_string_pretty(&api)?;
    let spec_yaml = serde_yml::to_string(&api)?;

    let router = router
        .route(
            "/list/openapi/json",
            axum::routing::get(|| async move {
                (
                    StatusCode::OK,
                    [("content-type", "application/json")],
                    spec_json,
                )
            }),
        )
        .route(
            "/list/openapi/yaml",
            axum::routing::get(|| async move {
                (StatusCode::OK, [("content-type", "text/yaml")], spec_yaml)
            }),
        )
        .route(
            "/list/graphql/graphql",
            axum::routing::get(|| async move {
                axum::response::Html(
                    GraphiQLSource::build()
                        .endpoint("/list/graphql/graphql")
                        .finish(),
                )
            })
            .post_service(GraphQL::new(schema)),
        )
        .route(
            "/list/graphql/playground",
            axum::routing::get(|| async move {
                (
                    StatusCode::OK,
                    [("content-type", "text/html")],
                    playground_source(GraphQLPlaygroundConfig::new("/list/graphql/graphql")),
                )
            }),
        );

    let host = &app.config.host;

    let addr: SocketAddr = format_sstr!("{host}:{port}").parse()?;
    debug!("{addr:?}");
    let listener = TcpListener::bind(&addr).await?;
    axum::serve(listener, router.into_make_service()).await?;

    Ok(())
}

#[cfg(test)]
mod test {
    use anyhow::Error;
    use stack_string::format_sstr;

    use crate::movie_queue_app::run_app;
    use movie_collection_lib::{config::Config, pgpool::PgPool, trakt_connection::TraktConnection};

    #[tokio::test]
    async fn test_run_app() -> Result<(), Error> {
        let config = Config::with_config()?;
        let pool = PgPool::new(&config.pgurl)?;
        let trakt = TraktConnection::new(config.clone());

        let test_port = 12345;
        tokio::task::spawn({
            let config = config.clone();
            async move {
                env_logger::init();
                run_app(config, pool, trakt, test_port).await.unwrap()
            }
        });
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;

        let client = reqwest::Client::new();

        let url = format_sstr!("http://localhost:{test_port}/list/openapi/yaml");
        let spec_yaml = client
            .get(url.as_str())
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;

        tokio::fs::write("../scripts/openapi.yaml", &spec_yaml).await?;
        Ok(())
    }
}

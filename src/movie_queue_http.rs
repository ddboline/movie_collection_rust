#![allow(clippy::used_underscore_binding)]
#![allow(clippy::needless_pass_by_value)]

use movie_collection_http::movie_queue_app::start_app;

#[tokio::main]
async fn main() {
    env_logger::init();

    start_app().await.unwrap();
}

#![allow(clippy::used_underscore_binding)]
#![allow(clippy::needless_pass_by_value)]

use movie_collection_http::movie_queue_app::start_app;
use movie_collection_lib::config::Config;

#[actix_rt::main]
async fn main() {
    env_logger::init();

    let config = Config::with_config().expect("Config init failed");
    start_app(config).await.unwrap();
}

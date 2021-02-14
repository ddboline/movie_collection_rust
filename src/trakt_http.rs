#![allow(clippy::used_underscore_binding)]
#![allow(clippy::needless_pass_by_value)]

use trakt_http::app::start_app;

#[tokio::main]
async fn main() {
    env_logger::init();

    start_app().await.unwrap();
}

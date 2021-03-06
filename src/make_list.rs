#![allow(clippy::used_underscore_binding)]

use anyhow::Error;
use stdout_channel::StdoutChannel;

use movie_collection_lib::make_list::make_list;

#[tokio::main]
async fn main() -> Result<(), Error> {
    env_logger::init();
    let stdout = StdoutChannel::new();

    match make_list(&stdout).await {
        Ok(_) => {}
        Err(e) => {
            if e.to_string().contains("Broken pipe") {
            } else {
                panic!("{}", e)
            }
        }
    }
    stdout.close().await
}

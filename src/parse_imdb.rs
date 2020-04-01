use anyhow::Error;
use structopt::StructOpt;

use movie_collection_lib::{
    movie_collection::MovieCollection,
    parse_imdb::{ParseImdb, ParseImdbOptions},
    stdout_channel::StdoutChannel,
};

async fn parse_imdb_parser() -> Result<(), Error> {
    let stdout = StdoutChannel::new();
    let task = stdout.clone().spawn_stdout_task();
    let opts = ParseImdbOptions::from_args();

    let mc = MovieCollection::new();
    let pi = ParseImdb::with_pool(&mc.pool)?;

    let output = pi.parse_imdb_worker(&opts).await?;

    for line in output {
        stdout.send(line.join(" "))?;
    }
    stdout.close().await;
    task.await?
}

#[tokio::main]
async fn main() {
    env_logger::init();

    match parse_imdb_parser().await {
        Ok(_) => (),
        Err(e) => {
            if e.to_string().contains("Broken pipe") {
            } else {
                panic!("{}", e)
            }
        }
    }
}

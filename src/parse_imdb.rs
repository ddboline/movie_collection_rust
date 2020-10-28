#![allow(clippy::used_underscore_binding)]

use anyhow::Error;
use structopt::StructOpt;

use movie_collection_lib::{
    config::Config,
    movie_collection::MovieCollection,
    parse_imdb::{ParseImdb, ParseImdbOptions},
};

async fn parse_imdb_parser() -> Result<(), Error> {
    let opts = ParseImdbOptions::from_args();
    let config = Config::with_config()?;
    let mc = MovieCollection::new(&config);
    let pi = ParseImdb::with_pool(&config, &mc.pool)?;

    let output: Vec<_> = pi
        .parse_imdb_worker(&opts)
        .await?
        .into_iter()
        .map(|x| x.join(" "))
        .collect();

    mc.stdout.send(output.join("\n"));

    mc.stdout.close().await
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

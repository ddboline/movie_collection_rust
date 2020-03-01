use anyhow::Error;
use std::io::{stdout, Write};
use structopt::StructOpt;

use movie_collection_lib::{
    movie_collection::MovieCollection,
    parse_imdb::{ParseImdb, ParseImdbOptions},
};

async fn parse_imdb_parser() -> Result<(), Error> {
    let opts = ParseImdbOptions::from_args();

    let mc = MovieCollection::new();
    let pi = ParseImdb::with_pool(&mc.pool)?;

    let output = pi.parse_imdb_worker(&opts).await?;

    let stdout = stdout();

    for line in output {
        writeln!(stdout.lock(), "{}", line.join(" "))?;
    }

    Ok(())
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

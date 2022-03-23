use anyhow::Error;
use stack_string::StackString;
use stdout_channel::StdoutChannel;
use structopt::StructOpt;

use movie_collection_lib::{
    config::Config,
    movie_collection::MovieCollection,
    parse_imdb::{ParseImdb, ParseImdbOptions},
    pgpool::PgPool,
};

async fn parse_imdb_parser() -> Result<(), Error> {
    let opts = ParseImdbOptions::from_args();
    let config = Config::with_config()?;
    let pool = PgPool::new(&config.pgurl);
    let stdout = StdoutChannel::new();

    let mc = MovieCollection::new(&config, &pool, &stdout);
    let pi = ParseImdb::new(&config, &pool, &stdout);

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
            let e = StackString::from_display(e);
            if e.contains("Broken pipe") {
            } else {
                panic!("{}", e);
            }
        }
    }
}

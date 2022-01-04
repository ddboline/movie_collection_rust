#![allow(clippy::used_underscore_binding)]

use anyhow::Error;
use stack_string::StackString;
use stdout_channel::StdoutChannel;
use structopt::StructOpt;

use movie_collection_lib::{
    config::Config, movie_collection::MovieCollection, pgpool::PgPool, tv_show_source::TvShowSource,
};

#[derive(StructOpt)]
/// Query and Parse Video Collection
struct FindNewEpisodesOpt {
    /// Restrict Source (possible values: all, netflix, hulu, amazon)
    #[structopt(long, short)]
    source: Option<TvShowSource>,

    /// Only Show Some Shows
    shows: Vec<StackString>,
}

async fn find_new_episodes() -> Result<(), Error> {
    let opts = FindNewEpisodesOpt::from_args();
    let config = Config::with_config()?;
    let pool = PgPool::new(&config.pgurl);
    let stdout = StdoutChannel::new();

    let source = if opts.shows.is_empty() {
        opts.source
    } else {
        Some(TvShowSource::All)
    };

    let mc = MovieCollection::new(&config, &pool, &stdout);

    let output = mc.find_new_episodes(source, &opts.shows).await?;

    for epi in output {
        stdout.send(StackString::from_display(epi).unwrap());
    }
    stdout.close().await
}

#[tokio::main]
async fn main() {
    env_logger::init();

    match find_new_episodes().await {
        Ok(_) => (),
        Err(e) => {
            let e = StackString::from_display(e).unwrap();
            if e.contains("Broken pipe") {
            } else {
                panic!("{}", e);
            }
        }
    }
}

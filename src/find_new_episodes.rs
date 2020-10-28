#![allow(clippy::used_underscore_binding)]

use anyhow::Error;
use stack_string::StackString;
use structopt::StructOpt;

use movie_collection_lib::{
    config::Config, movie_collection::MovieCollection, tv_show_source::TvShowSource,
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

    let source = if opts.shows.is_empty() {
        opts.source
    } else {
        Some(TvShowSource::All)
    };
    let config = Config::with_config()?;
    let mc = MovieCollection::new(&config);

    let output = mc.find_new_episodes(source, &opts.shows).await?;

    for epi in output {
        mc.stdout.send(epi.to_string());
    }
    mc.stdout.close().await
}

#[tokio::main]
async fn main() {
    env_logger::init();

    match find_new_episodes().await {
        Ok(_) => (),
        Err(e) => {
            if e.to_string().contains("Broken pipe") {
            } else {
                panic!("{}", e)
            }
        }
    }
}

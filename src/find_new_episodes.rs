use anyhow::Error;
use std::{io, io::Write};
use structopt::StructOpt;

use movie_collection_lib::{movie_collection::MovieCollection, tv_show_source::TvShowSource};

#[derive(StructOpt)]
/// Query and Parse Video Collection
struct FindNewEpisodesOpt {
    /// Restrict Source (possible values: all, netflix, hulu, amazon)
    #[structopt(long, short)]
    source: Option<TvShowSource>,

    /// Only Show Some Shows
    shows: Vec<String>,
}

async fn find_new_episodes() -> Result<(), Error> {
    let opts = FindNewEpisodesOpt::from_args();

    let source = if opts.shows.is_empty() {
        opts.source
    } else {
        Some(TvShowSource::All)
    };

    let mc = MovieCollection::new();

    let output = mc.find_new_episodes(&source, &opts.shows).await?;

    let stdout = io::stdout();

    for epi in output {
        writeln!(stdout.lock(), "{}", epi)?;
    }

    Ok(())
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

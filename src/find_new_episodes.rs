use anyhow::Error;
use structopt::StructOpt;

use movie_collection_lib::{
    movie_collection::MovieCollection, stack_string::StackString, tv_show_source::TvShowSource,
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

    let mc = MovieCollection::new();
    let task = mc.stdout.spawn_stdout_task();

    let output = mc.find_new_episodes(source, &opts.shows).await?;

    for epi in output {
        mc.stdout.send(epi.to_string().into())?;
    }
    mc.stdout.close().await?;
    task.await?
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

#![allow(clippy::used_underscore_binding)]

use anyhow::Error;
use futures::future::try_join_all;
use stack_string::StackString;
use std::path::Path;
use stdout_channel::StdoutChannel;
use structopt::StructOpt;

use movie_collection_lib::{
    config::Config, movie_collection::MovieCollection, pgpool::PgPool, utils::get_video_runtime,
};

#[derive(StructOpt)]
/// Collection Query/Parser
///
/// Query and Parse Video Collection
struct MakeCollectionOpts {
    /// Parse collection for new videos
    #[structopt(short, long)]
    parse: bool,

    /// Compute Runtime
    #[structopt(short, long)]
    time: bool,

    /// Shows to display
    shows: Vec<StackString>,
}

async fn make_collection() -> Result<(), Error> {
    let opts = MakeCollectionOpts::from_args();
    let config = Config::with_config()?;
    let do_parse = opts.parse;
    let do_time = opts.time;
    let stdout = StdoutChannel::new();
    let pool = PgPool::new(&config.pgurl);

    let mc = MovieCollection::new(&config, &pool, &stdout);
    if do_parse {
        mc.make_collection().await?;
        mc.fix_collection_show_id().await?;
    } else {
        let shows = mc.search_movie_collection(&opts.shows).await?;
        if do_time {
            let futures = shows.into_iter().map(|result| async move {
                let path = Path::new(result.path.as_str());
                let timeval = get_video_runtime(&path).await?;
                Ok(format!("{} {}", timeval, result))
            });
            let shows: Result<Vec<_>, Error> = try_join_all(futures).await;
            stdout.send(shows?.join("\n"));
        } else {
            for show in shows {
                stdout.send(show.to_string());
            }
        }
    }
    stdout.close().await
}

#[tokio::main]
async fn main() {
    env_logger::init();

    match make_collection().await {
        Ok(_) => {}
        Err(e) => {
            if !e.to_string().contains("Broken pipe") {
                panic!("{}", e)
            }
        }
    }
}

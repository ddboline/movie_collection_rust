use anyhow::Error;
use futures::future::try_join_all;
use structopt::StructOpt;
use tokio::task::spawn_blocking;

use movie_collection_lib::{movie_collection::MovieCollection, utils::get_video_runtime};
use movie_collection_lib::stack_string::StackString;

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

    let do_parse = opts.parse;
    let do_time = opts.time;

    let mc = MovieCollection::new();
    let task = mc.stdout.spawn_stdout_task();
    if do_parse {
        mc.make_collection().await?;
        mc.fix_collection_show_id().await?;
    } else {
        let shows = mc.search_movie_collection(&opts.shows).await?;
        if do_time {
            let futures = shows.into_iter().map(|result| async move {
                let path = result.path.clone();
                let timeval = spawn_blocking(move || get_video_runtime(path.as_str())).await??;
                Ok((timeval, result))
            });
            let shows: Result<Vec<_>, Error> = try_join_all(futures).await;
            for (timeval, show) in shows? {
                mc.stdout.send(format!("{} {}", timeval, show).into())?;
            }
        } else {
            for show in shows {
                mc.stdout.send(show.to_string().into())?;
            }
        }
    }
    mc.stdout.close().await?;
    task.await?
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

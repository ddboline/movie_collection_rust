use anyhow::Error;
use futures::future::try_join_all;
use std::io;
use std::io::Write;
use structopt::StructOpt;
use tokio::task::spawn_blocking;

use movie_collection_lib::movie_collection::MovieCollection;
use movie_collection_lib::utils::get_video_runtime;

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
    shows: Vec<String>,
}

async fn make_collection() -> Result<(), Error> {
    let opts = MakeCollectionOpts::from_args();

    let do_parse = opts.parse;
    let do_time = opts.time;

    let stdout = io::stdout();

    let mc = MovieCollection::new();
    if do_parse {
        mc.make_collection().await?;
        mc.fix_collection_show_id().await?;
    } else {
        let shows = mc.search_movie_collection(&opts.shows).await?;
        if do_time {
            let futures: Vec<_> = shows
                .into_iter()
                .map(|result| async move {
                    let path = result.path.clone();
                    let timeval = spawn_blocking(move || get_video_runtime(&path)).await??;
                    Ok((timeval, result))
                })
                .collect();
            let shows: Result<Vec<_>, Error> = try_join_all(futures).await;
            for (timeval, show) in shows? {
                writeln!(stdout.lock(), "{} {}", timeval, show)?;
            }
        } else {
            for show in shows {
                writeln!(stdout.lock(), "{}", show)?;
            }
        }
    }
    Ok(())
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

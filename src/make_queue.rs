use anyhow::Error;
use structopt::StructOpt;

use movie_collection_lib::make_queue::{make_queue_worker, PathOrIndex};

#[derive(StructOpt)]
/// Parse IMDB.com
struct MakeQueueOpts {
    #[structopt(long, short, parse(from_os_str))]
    /// Add files(s) to queue
    add: Vec<PathOrIndex>,
    #[structopt(long, short, parse(from_os_str))]
    /// Remove entries by index OR filename
    remove: Vec<PathOrIndex>,
    #[structopt(long, short)]
    /// Compute Runtime of Files
    time: bool,
    #[structopt(long, short)]
    /// Display information about tv shows in queue
    shows: bool,
    /// String patterns to filter on
    patterns: Vec<String>,
}

async fn make_queue() -> Result<(), Error> {
    let opts = MakeQueueOpts::from_args();

    let patterns: Vec<_> = opts.patterns.iter().map(String::as_str).collect();

    make_queue_worker(&opts.add, &opts.remove, opts.time, &patterns, opts.shows).await
}

#[tokio::main]
async fn main() {
    env_logger::init();

    match make_queue().await {
        Ok(_) => (),
        Err(e) => {
            if e.to_string().contains("Broken pipe") {
            } else {
                panic!("{}", e)
            }
        }
    }
}

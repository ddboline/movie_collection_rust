#![allow(clippy::used_underscore_binding)]

use anyhow::Error;
use stack_string::StackString;
use stdout_channel::StdoutChannel;
use structopt::StructOpt;

use movie_collection_lib::{
    config::Config,
    make_queue::{make_queue_worker, PathOrIndex},
};

#[derive(StructOpt)]
/// Manage Video Queue
struct MakeQueueOpts {
    /// Add files(s) to queue
    #[structopt(long, short, parse(from_os_str))]
    add: Vec<PathOrIndex>,

    /// Remove entries by index OR filename
    #[structopt(long, short, parse(from_os_str))]
    remove: Vec<PathOrIndex>,

    /// Compute Runtime of Files
    #[structopt(long, short)]
    time: bool,

    /// Display information about tv shows in queue
    #[structopt(long, short)]
    shows: bool,

    /// String patterns to filter on
    patterns: Vec<StackString>,
}

async fn make_queue() -> Result<(), Error> {
    let opts = MakeQueueOpts::from_args();
    let config = Config::with_config()?;
    let stdout = StdoutChannel::new();
    let patterns: Vec<_> = opts.patterns.iter().map(StackString::as_str).collect();

    make_queue_worker(
        &config,
        &opts.add,
        &opts.remove,
        opts.time,
        &patterns,
        opts.shows,
        &stdout,
    )
    .await?;
    stdout.close().await
}

#[tokio::main]
async fn main() {
    env_logger::init();

    match make_queue().await {
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

use anyhow::Error;
use clap::Parser;
use stack_string::StackString;
use std::ffi::OsStr;
use stdout_channel::StdoutChannel;

use movie_collection_lib::{
    config::Config,
    make_queue::{make_queue_worker, PathOrIndex},
};

#[allow(clippy::unnecessary_wraps)]
fn parse_path_or_index(s: &str) -> Result<PathOrIndex, String> {
    let p: &OsStr = s.as_ref();
    Ok(p.into())
}

#[derive(Parser, Clone)]
/// Manage Video Queue
struct MakeQueueOpts {
    /// Add files(s) to queue
    #[clap(long, short, value_parser = parse_path_or_index)]
    add: Vec<PathOrIndex>,

    /// Remove entries by index OR filename
    #[clap(long, short, value_parser = parse_path_or_index)]
    remove: Vec<PathOrIndex>,

    /// Compute Runtime of Files
    #[clap(long, short)]
    time: bool,

    /// Display information about tv shows in queue
    #[clap(long, short)]
    shows: bool,

    /// String patterns to filter on
    patterns: Vec<StackString>,
}

async fn make_queue() -> Result<(), Error> {
    let opts = MakeQueueOpts::parse();
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
    stdout.close().await.map_err(Into::into)
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

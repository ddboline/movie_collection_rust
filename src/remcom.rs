#![allow(clippy::used_underscore_binding)]

use anyhow::Error;
use std::path::PathBuf;
use structopt::StructOpt;

use movie_collection_lib::{
    config::Config, stdout_channel::StdoutChannel, transcode_service::remcom,
};

#[derive(StructOpt)]
/// Create script to copy files, push job to queue
struct RemcomOpts {
    /// Directory
    #[structopt(long, short)]
    directory: Option<PathBuf>,

    #[structopt(long, short)]
    unwatched: bool,

    files: Vec<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    env_logger::init();
    let opts = RemcomOpts::from_args();
    let stdout = StdoutChannel::new();
    let config = Config::with_config()?;

    match remcom(
        opts.files,
        opts.directory.as_deref(),
        opts.unwatched,
        &config,
        &stdout,
    )
    .await
    {
        Ok(_) => (),
        Err(e) => {
            if e.to_string().contains("Broken pipe") {
            } else {
                panic!("{}", e)
            }
        }
    }
    Ok(())
}

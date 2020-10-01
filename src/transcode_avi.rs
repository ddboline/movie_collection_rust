#![allow(clippy::used_underscore_binding)]

use anyhow::Error;
use std::path::PathBuf;
use structopt::StructOpt;

use movie_collection_lib::{
    config::Config, stdout_channel::StdoutChannel, transcode_service::transcode_avi,
};

#[derive(StructOpt)]
struct TranscodeAviOpts {
    files: Vec<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    env_logger::init();
    let stdout = StdoutChannel::new();
    let config = Config::with_config()?;

    let opts = TranscodeAviOpts::from_args();

    match transcode_avi(&config, &stdout, &opts.files).await {
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

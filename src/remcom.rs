#![allow(clippy::used_underscore_binding)]

use anyhow::Error;
use std::path::PathBuf;
use structopt::StructOpt;

use movie_collection_lib::{
    config::Config,
    stdout_channel::StdoutChannel,
    transcode_service::{TranscodeService, TranscodeServiceRequest},
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

async fn remcom() -> Result<(), Error> {
    let opts = RemcomOpts::from_args();
    let stdout = StdoutChannel::new();
    let config = Config::with_config()?;

    let remcom_service = TranscodeService::new(config.clone(), &config.remcom_queue);

    for file in opts.files {
        let payload = TranscodeServiceRequest::create_remcom_request(
            &config,
            &file,
            opts.directory.as_deref(),
            opts.unwatched,
        )
        .await?;
        remcom_service.publish_transcode_job(&payload).await?;
        stdout.send(format!("script {:?}", payload));
    }
    stdout.close().await
}

#[tokio::main]
async fn main() {
    env_logger::init();

    match remcom().await {
        Ok(_) => (),
        Err(e) => {
            if e.to_string().contains("Broken pipe") {
            } else {
                panic!("{}", e)
            }
        }
    }
}

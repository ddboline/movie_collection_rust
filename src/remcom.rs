#![allow(clippy::used_underscore_binding)]

use anyhow::Error;
use std::path::{Path, PathBuf};
use stdout_channel::StdoutChannel;
use structopt::StructOpt;

use movie_collection_lib::{
    config::Config,
    transcode_service::{TranscodeService, TranscodeServiceRequest},
};
use transcode_lib::transcode_channel::TranscodeChannel;

pub async fn remcom(
    files: impl IntoIterator<Item = impl AsRef<Path>>,
    directory: Option<impl AsRef<Path>>,
    unwatched: bool,
    config: &Config,
    stdout: &StdoutChannel,
) -> Result<(), Error> {
    let remcom_service = TranscodeService::new(config.clone(), &config.remcom_queue);

    for file in files {
        let payload = TranscodeServiceRequest::create_remcom_request(
            &config,
            file.as_ref(),
            directory.as_ref(),
            unwatched,
        )
        .await?;
        remcom_service
            .publish_transcode_job(&payload, |data| async move {
                let remcom_channel = TranscodeChannel::open_channel().await?;
                remcom_channel.init(&config.remcom_queue).await?;
                remcom_channel.publish(&config.transcode_queue, data).await
            })
            .await?;
        stdout.send(format!("script {:?}", payload));
    }
    stdout.close().await
}

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

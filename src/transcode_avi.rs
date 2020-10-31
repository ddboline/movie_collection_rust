#![allow(clippy::used_underscore_binding)]

use anyhow::Error;
use std::path::{Path, PathBuf};
use stdout_channel::StdoutChannel;
use structopt::StructOpt;

use movie_collection_lib::{
    config::Config,
    transcode_service::{movie_dir, TranscodeService, TranscodeServiceRequest},
};
use transcode_lib::transcode_channel::TranscodeChannel;

pub async fn transcode_avi(
    config: &Config,
    stdout: &StdoutChannel,
    files: impl IntoIterator<Item = impl AsRef<Path>>,
) -> Result<(), Error> {
    let transcode_service = TranscodeService::new(config.clone(), &config.transcode_queue);

    for path in files {
        let path = path.as_ref();
        let movie_path = movie_dir(config);
        let path = if path.exists() {
            path.to_path_buf()
        } else {
            movie_path.join(path)
        }
        .canonicalize()?;

        if !path.exists() {
            panic!("file doesn't exist {}", path.to_string_lossy());
        }
        let payload = TranscodeServiceRequest::create_transcode_request(&config, &path)?;

        transcode_service
            .publish_transcode_job(&payload, |data| async move {
                let transcode_channel = TranscodeChannel::open_channel().await?;
                transcode_channel.init(&config.transcode_queue).await?;
                transcode_channel
                    .publish(&config.transcode_queue, data)
                    .await
            })
            .await?;
        stdout.send(format!("script {:?}", payload));
    }
    stdout.close().await
}

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

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
struct TranscodeAviOpts {
    files: Vec<PathBuf>,
}

async fn transcode_avi() -> Result<(), Error> {
    let stdout = StdoutChannel::new();
    let config = Config::with_config()?;
    let task = stdout.spawn_stdout_task();

    let opts = TranscodeAviOpts::from_args();
    let transcode_service = TranscodeService::new(config.clone(), &config.transcode_queue);
    transcode_service.init().await?;

    for path in opts.files {
        let movie_path = config.home_dir.join("Documents").join("movies");
        let path = if path.exists() {
            path
        } else {
            movie_path.join(path)
        }
        .canonicalize()?;

        if !path.exists() {
            panic!("file doesn't exist {}", path.to_string_lossy());
        }
        let payload = TranscodeServiceRequest::create_transcode_request(&config, &path)?;
        transcode_service.publish_transcode_job(&payload).await?;
        stdout.send(format!("script {:?}", payload).into())?;
    }
    stdout.close().await?;
    task.await?
}

#[tokio::main]
async fn main() {
    env_logger::init();

    match transcode_avi().await {
        Ok(_) => (),
        Err(e) => {
            if e.to_string().contains("Broken pipe") {
            } else {
                panic!("{}", e)
            }
        }
    }
}

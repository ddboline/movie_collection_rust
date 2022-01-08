#![allow(clippy::used_underscore_binding)]

use anyhow::Error;
use stack_string::StackString;
use std::path::{Path, PathBuf};
use stdout_channel::StdoutChannel;
use structopt::StructOpt;
use tokio::fs;

use movie_collection_lib::{
    config::Config,
    pgpool::PgPool,
    transcode_service::{movie_dir, TranscodeService, TranscodeServiceRequest},
};
use transcode_lib::transcode_channel::TranscodeChannel;

async fn transcode_avi(
    transcode_service: &TranscodeService,
    config: &Config,
    stdout: &StdoutChannel<StackString>,
    files: impl IntoIterator<Item = impl AsRef<Path>>,
) -> Result<(), Error> {
    for path in files {
        let path = path.as_ref();
        let movie_path = movie_dir(config);
        let path = if path.exists() {
            path.to_path_buf()
        } else {
            movie_path.join(path)
        }
        .canonicalize()?;
        assert!(
            path.exists(),
            "file doesn't exist {}",
            path.to_string_lossy()
        );
        let payload = TranscodeServiceRequest::create_transcode_request(config, &path)?;
        publish_single(transcode_service, &payload).await?;
        stdout.send(format!("script {:?}", payload));
    }
    stdout.close().await
}

async fn publish_single(
    transcode_service: &TranscodeService,
    payload: &TranscodeServiceRequest,
) -> Result<(), Error> {
    transcode_service
        .publish_transcode_job(payload, |data| async move {
            let transcode_channel = TranscodeChannel::open_channel().await?;
            transcode_channel.init(&transcode_service.queue).await?;
            transcode_channel
                .publish(&transcode_service.queue, data)
                .await
        })
        .await?;
    Ok(())
}

async fn transcode_single(
    transcode_service: &TranscodeService,
    request_file: &Path,
) -> Result<(), Error> {
    let data = fs::read(request_file).await?;
    let payload = serde_json::from_slice(&data)?;
    publish_single(transcode_service, &payload).await?;
    Ok(())
}

#[derive(StructOpt)]
struct TranscodeAviOpts {
    #[structopt(short = "f", long)]
    request_file: Option<PathBuf>,
    files: Vec<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    env_logger::init();
    let stdout = StdoutChannel::new();
    let config = Config::with_config()?;
    let pool = PgPool::new(&config.pgurl);

    let opts = TranscodeAviOpts::from_args();

    let transcode_service = TranscodeService::new(&config, &config.transcode_queue, &pool, &stdout);

    if let Some(request_file) = &opts.request_file {
        match transcode_single(&transcode_service, request_file).await {
            Ok(_) => (),
            Err(e) => {
                let e = StackString::from_display(e);
                if e.contains("Broken pipe") {
                } else {
                    panic!("{}", e);
                }
            }
        }
        return Ok(());
    }

    match transcode_avi(&transcode_service, &config, &stdout, &opts.files).await {
        Ok(_) => (),
        Err(e) => {
            let e = StackString::from_display(e);
            if e.contains("Broken pipe") {
            } else {
                panic!("{}", e);
            }
        }
    }
    Ok(())
}

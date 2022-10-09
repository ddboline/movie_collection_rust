use anyhow::Error;
use clap::Parser;
use stack_string::StackString;
use std::path::{Path, PathBuf};
use stdout_channel::StdoutChannel;
use tokio::fs;

use movie_collection_lib::{
    config::Config,
    pgpool::PgPool,
    transcode_service::{TranscodeService, TranscodeServiceRequest},
};
use transcode_lib::transcode_channel::TranscodeChannel;

async fn remcom(
    remcom_service: &TranscodeService,
    files: impl IntoIterator<Item = impl AsRef<Path>>,
    directory: Option<impl AsRef<Path>>,
    unwatched: bool,
    config: &Config,
    stdout: &StdoutChannel<StackString>,
) -> Result<(), Error> {
    for file in files {
        let payload = TranscodeServiceRequest::create_remcom_request(
            config,
            file.as_ref(),
            directory.as_ref(),
            unwatched,
        )
        .await?;
        publish_single(remcom_service, &payload).await?;
        stdout.send(format!("script {payload:?}"));
    }
    stdout.close().await.map_err(Into::into)
}

async fn publish_single(
    remcom_service: &TranscodeService,
    payload: &TranscodeServiceRequest,
) -> Result<(), Error> {
    remcom_service
        .publish_transcode_job(payload, |data| async move {
            let remcom_channel = TranscodeChannel::open_channel().await?;
            remcom_channel.init(&remcom_service.queue).await?;
            remcom_channel.publish(&remcom_service.queue, data).await
        })
        .await?;
    Ok(())
}

async fn remcom_single(
    remcom_service: &TranscodeService,
    request_file: &Path,
) -> Result<(), Error> {
    let data = fs::read(request_file).await?;
    let payload = serde_json::from_slice(&data)?;
    publish_single(remcom_service, &payload).await?;
    Ok(())
}

#[derive(Parser)]
/// Create script to copy files, push job to queue
struct RemcomOpts {
    #[clap(short = 'f', long)]
    request_file: Option<PathBuf>,

    /// Directory
    #[clap(long, short)]
    directory: Option<PathBuf>,

    #[clap(long, short)]
    unwatched: bool,

    files: Vec<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    env_logger::init();
    let opts = RemcomOpts::from_args();
    let stdout = StdoutChannel::new();
    let config = Config::with_config()?;
    let pool = PgPool::new(&config.pgurl);

    let remcom_service = TranscodeService::new(&config, &config.remcom_queue, &pool, &stdout);

    if let Some(request_file) = opts.request_file {
        match remcom_single(&remcom_service, &request_file).await {
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

    match remcom(
        &remcom_service,
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
            let e = StackString::from_display(e);
            if e.contains("Broken pipe") {
            } else {
                panic!("{}", e);
            }
        }
    }
    Ok(())
}

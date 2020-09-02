#![allow(clippy::used_underscore_binding)]

use anyhow::Error;
use tokio::task::spawn;

use movie_collection_lib::{config::Config, transcode_service::TranscodeService};

#[tokio::main]
async fn main() -> Result<(), Error> {
    env_logger::init();
    let config = Config::with_config().unwrap();
    let transcode_service = TranscodeService::new(config.clone(), &config.transcode_queue);
    let remcom_service = TranscodeService::new(config.clone(), &config.remcom_queue);

    let transcode_task = spawn(async move { transcode_service.read_transcode_job().await });
    let remcom_task = spawn(async move { remcom_service.read_transcode_job().await });

    transcode_task.await??;
    remcom_task.await??;

    Ok(())
}

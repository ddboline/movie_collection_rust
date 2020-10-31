#![allow(clippy::used_underscore_binding)]

use anyhow::Error;
use tokio::task::spawn;
use transcode_lib::transcode_channel::TranscodeChannel;

use movie_collection_lib::{config::Config, transcode_service::TranscodeService};

#[tokio::main]
async fn main() -> Result<(), Error> {
    env_logger::init();
    let config = Config::with_config().unwrap();
    let transcode_service = TranscodeService::new(config.clone(), &config.transcode_queue);
    let transcode_channel = TranscodeChannel::open_channel().await?;
    transcode_channel.init(&config.transcode_queue).await?;
    let remcom_service = TranscodeService::new(config.clone(), &config.remcom_queue);
    let remcom_channel = TranscodeChannel::open_channel().await?;
    remcom_channel.init(&config.remcom_queue).await?;

    let transcode_task = spawn(async move {
        transcode_channel
            .read_transcode_job(&transcode_service.queue, |data| {
                let trans = transcode_service.clone();
                async move { trans.process_data(&data).await }
            })
            .await
    });
    let remcom_task = spawn(async move {
        remcom_channel
            .read_transcode_job(&remcom_service.queue, |data| {
                let remcom = remcom_service.clone();
                async move { remcom.process_data(&data).await }
            })
            .await
    });

    transcode_task.await??;
    remcom_task.await??;

    Ok(())
}

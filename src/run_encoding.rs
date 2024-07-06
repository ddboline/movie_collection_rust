use anyhow::Error;
use stdout_channel::StdoutChannel;
use tokio::task::spawn;
use transcode_lib::transcode_channel::TranscodeChannel;

use movie_collection_lib::{config::Config, pgpool::PgPool, transcode_service::TranscodeService};

#[tokio::main]
async fn main() -> Result<(), Error> {
    env_logger::init();
    let config = Config::with_config().unwrap();
    let pool = PgPool::new(&config.pgurl)?;
    let stdout = StdoutChannel::new();

    let transcode_service = TranscodeService::new(&config, &config.transcode_queue, &pool, &stdout);
    let transcode_channel = TranscodeChannel::open_channel().await?;
    transcode_channel.init(&transcode_service.queue).await?;
    let remcom_service = TranscodeService::new(&config, &config.remcom_queue, &pool, &stdout);
    let remcom_channel = TranscodeChannel::open_channel().await?;
    remcom_channel.init(&remcom_service.queue).await?;

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
    stdout.close().await?;

    Ok(())
}

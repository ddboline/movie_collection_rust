use anyhow::{format_err, Error};
use deadpool_lapin::Config as LapinConfig;
use derive_more::{Deref, DerefMut};
use futures::StreamExt;
use lapin::{
    options::{
        BasicAckOptions, BasicConsumeOptions, BasicPublishOptions, QueueDeclareOptions,
        QueueDeleteOptions, QueuePurgeOptions,
    },
    types::FieldTable,
    BasicProperties, Channel, Queue,
};
use log::{debug, error};
use serde::de::DeserializeOwned;
use std::future::Future;

#[derive(Clone, Deref, DerefMut)]
pub struct TranscodeChannel(Channel);

impl TranscodeChannel {
    pub async fn open_channel() -> Result<Self, Error> {
        let cfg = LapinConfig::default();
        let pool = cfg.create_pool(None)?;
        let conn = pool.get().await?;
        conn.create_channel().await.map_err(Into::into).map(Self)
    }

    pub async fn init(&self, queue: &str) -> Result<Queue, Error> {
        let queue = self
            .queue_declare(queue, QueueDeclareOptions::default(), FieldTable::default())
            .await?;
        Ok(queue)
    }

    pub async fn cleanup(&self, queue: &str) -> Result<u32, Error> {
        self.queue_purge(queue, QueuePurgeOptions::default())
            .await?;
        self.queue_delete(queue, QueueDeleteOptions::default())
            .await
            .map_err(Into::into)
    }

    pub async fn publish(&self, queue: &str, payload: Vec<u8>) -> Result<(), Error> {
        self.basic_publish(
            "",
            queue,
            BasicPublishOptions::default(),
            &payload,
            BasicProperties::default(),
        )
        .await?;
        Ok(())
    }

    pub async fn read_transcode_job<F, T>(&self, queue: &str, process_data: F) -> Result<(), Error>
    where
        F: Fn(Vec<u8>) -> T,
        T: Future<Output = Result<(), Error>>,
    {
        let mut consumer = self
            .basic_consume(
                queue,
                queue,
                BasicConsumeOptions::default(),
                FieldTable::default(),
            )
            .await?;
        while let Some(delivery) = consumer.next().await {
            let mut delivery = delivery?;
            let mut data = Vec::new();
            std::mem::swap(&mut delivery.data, &mut data);
            match process_data(data).await {
                Ok(()) => debug!("process_data succeeded"),
                Err(e) => error!("process data failed {:?}", e),
            }
            delivery.ack(BasicAckOptions::default()).await?;
        }
        Ok(())
    }

    pub async fn get_single_job<T: DeserializeOwned>(&self, queue: &str) -> Result<T, Error> {
        let mut consumer = self
            .basic_consume(
                queue,
                queue,
                BasicConsumeOptions::default(),
                FieldTable::default(),
            )
            .await?;
        if let Some(delivery) = consumer.next().await {
            let delivery = delivery?;
            let payload: T = serde_json::from_slice(&delivery.data)?;
            delivery.ack(BasicAckOptions::default()).await?;
            Ok(payload)
        } else {
            Err(format_err!("No Messages?"))
        }
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Error;
    use std::path::Path;
    use stdout_channel::{MockStdout, StdoutChannel};
    use tokio::{
        fs,
        task::{spawn, JoinHandle},
    };

    use crate::transcode_channel::TranscodeChannel;
    use movie_collection_lib::{
        config::Config,
        pgpool::PgPool,
        transcode_service::{job_dir, JobType, TranscodeService, TranscodeServiceRequest},
    };

    #[tokio::test]
    #[ignore]
    async fn test_transcode_service() -> Result<(), Error> {
        let config = Config::with_config()?;
        let pool = PgPool::new(&config.pgurl)?;

        let mock_stdout = MockStdout::new();
        let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

        let test_queue = "test_queue";
        let channel = TranscodeChannel::open_channel().await?;
        let queue = channel.init(test_queue).await?;
        println!("{:?}", queue);
        let task: JoinHandle<Result<_, Error>> = spawn(async move {
            let result: TranscodeServiceRequest = channel.get_single_job(test_queue).await?;
            Ok(result)
        });
        let service = TranscodeService::new(&config, test_queue, &pool, &stdout);
        let req = TranscodeServiceRequest::new(
            JobType::Transcode,
            "test_prefix",
            &Path::new("test_input.mkv"),
            &Path::new("test_output.mp4"),
        );
        service
            .publish_transcode_job(&req, |data| async move {
                let transcode_channel = TranscodeChannel::open_channel().await?;
                transcode_channel.init(test_queue).await?;
                transcode_channel.publish(test_queue, data).await
            })
            .await?;
        let result = task.await??;
        assert_eq!(result, req);

        let transcode_channel = TranscodeChannel::open_channel().await?;
        transcode_channel.init(test_queue).await?;

        let result = transcode_channel.cleanup(test_queue).await?;
        println!("{}", result);
        let script_file = job_dir(&service.config)
            .join(&req.prefix)
            .with_extension("json");
        fs::remove_file(&script_file).await?;

        Ok(())
    }
}

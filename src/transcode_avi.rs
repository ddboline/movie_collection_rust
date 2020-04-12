use anyhow::Error;
use std::path::{Path, PathBuf};
use structopt::StructOpt;

use movie_collection_lib::{
    config::Config,
    stdout_channel::StdoutChannel,
    utils::{create_transcode_script, publish_transcode_job_to_queue},
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

    for path in opts.files {
        let movie_path = Path::new(config.home_dir.as_str())
            .join("Documents")
            .join("movies");
        let path = if path.exists() {
            path
        } else {
            movie_path.join(path)
        }
        .canonicalize()?;

        if !path.exists() {
            panic!("file doesn't exist {}", path.to_string_lossy());
        }
        create_transcode_script(&config, &path).and_then(|s| {
            stdout.send(format!("script {}", s).into())?;
            publish_transcode_job_to_queue(
                &s,
                config.transcode_queue.as_str(),
                config.transcode_queue.as_str(),
            )
        })?;
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

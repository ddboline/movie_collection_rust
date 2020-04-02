use anyhow::Error;
use std::path::PathBuf;
use structopt::StructOpt;

use movie_collection_lib::utils::remcom_single_file;
use movie_collection_lib::stdout_channel::StdoutChannel;

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

async fn remcom() -> Result<(), Error> {
    let opts = RemcomOpts::from_args();
    let stdout = StdoutChannel::new();
    let task = stdout.spawn_stdout_task();

    for file in opts.files {
        remcom_single_file(&file, opts.directory.as_deref(), opts.unwatched, &stdout)?;
    }
    stdout.close().await?;
    task.await?
}

#[tokio::main]
async fn main() {
    env_logger::init();

    match remcom().await {
        Ok(_) => (),
        Err(e) => {
            if e.to_string().contains("Broken pipe") {
            } else {
                panic!("{}", e)
            }
        }
    }
}

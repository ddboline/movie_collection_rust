use anyhow::Error;
use std::{
    io::{stdout, Write},
    path::{Path, PathBuf},
};
use structopt::StructOpt;

use movie_collection_lib::{
    config::Config,
    utils::{create_transcode_script, publish_transcode_job_to_queue},
};

#[derive(StructOpt)]
struct TranscodeAviOpts {
    files: Vec<PathBuf>,
}

fn transcode_avi() -> Result<(), Error> {
    let stdout = stdout();
    let config = Config::with_config()?;

    let opts = TranscodeAviOpts::from_args();

    for path in opts.files {
        let movie_path = Path::new(&config.home_dir).join("Documents").join("movies");
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
            writeln!(stdout.lock(), "script {}", s)?;
            publish_transcode_job_to_queue(&s, &config.transcode_queue, &config.transcode_queue)
        })?;
    }
    Ok(())
}

fn main() {
    env_logger::init();

    match transcode_avi() {
        Ok(_) => (),
        Err(e) => {
            if e.to_string().contains("Broken pipe") {
            } else {
                panic!("{}", e)
            }
        }
    }
}

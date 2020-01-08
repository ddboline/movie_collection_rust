use anyhow::{format_err, Error};
use clap::{App, Arg};
use log::error;
use std::io::{stdout, Write};
use std::path::{Path, PathBuf};

use movie_collection_lib::config::Config;
use movie_collection_lib::utils::{
    create_transcode_script, get_version_number, publish_transcode_job_to_queue,
};

fn transcode_avi() -> Result<(), Error> {
    let stdout = stdout();

    let config = Config::with_config()?;

    let matches = App::new("Transcode AVI")
        .version(get_version_number().as_str())
        .author("Daniel Boline <ddboline@gmail.com>")
        .about("Create script to do stuff")
        .arg(
            Arg::with_name("files")
                .multiple(true)
                .help("Files")
                .required(true),
        )
        .get_matches();

    matches
        .values_of("files")
        .ok_or_else(|| format_err!("No files given"))?
        .map(|f| {
            let path = PathBuf::from(f);

            let movie_path = format!("{}/Documents/movies", config.home_dir);

            let path = if path.exists() {
                path
            } else {
                Path::new(&movie_path).join(f)
            }
            .canonicalize()?;

            if !path.exists() {
                panic!("file doesn't exist {}", f);
            }

            create_transcode_script(&config, &path)
                .and_then(|s| {
                    writeln!(stdout.lock(), "script {}", s)?;
                    publish_transcode_job_to_queue(
                        &s,
                        &config.transcode_queue,
                        &config.transcode_queue,
                    )
                })
                .map_err(|e| {
                    error!("error {}", e);
                    e
                })
        })
        .collect()
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

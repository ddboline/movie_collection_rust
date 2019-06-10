use clap::{App, Arg};
use failure::{err_msg, Error};
use std::io::{stdout, Write};
use std::path::{Path, PathBuf};

use movie_collection_lib::common::config::Config;
use movie_collection_lib::common::utils::{
    create_transcode_script, get_version_number, publish_transcode_job_to_queue,
};

fn transcode_avi() -> Result<(), Error> {
    let stdout = stdout();

    let config = Config::with_config();

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

    let files = matches
        .values_of("files")
        .ok_or_else(|| err_msg("No files given"))?;
    for f in files {
        let path = PathBuf::from(f);

        let movie_path = format!("{}/Documents/movies", config.home_dir);

        let path = if !path.exists() {
            Path::new(&movie_path).join(f)
        } else {
            path
        }
        .canonicalize()?;

        if !path.exists() {
            panic!("file doesn't exist {}", f);
        }

        match create_transcode_script(&config, &path) {
            Ok(s) => {
                writeln!(stdout.lock(), "script {}", s)?;
                publish_transcode_job_to_queue(&s, "transcode_work_queue", "transcode_work_queue")?;
            }
            Err(e) => writeln!(stdout.lock(), "error {}", e)?,
        }
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

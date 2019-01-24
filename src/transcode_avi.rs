extern crate dotenv;
extern crate failure;
extern crate movie_collection_rust;

use clap::{App, Arg};
use failure::{err_msg, Error};
use std::io::{stdout, Write};
use std::path::{Path, PathBuf};

use movie_collection_rust::common::config::Config;
use movie_collection_rust::common::utils::{
    create_transcode_script, get_version_number, publish_transcode_job_to_queue,
};

fn transcode_avi() -> Result<(), Error> {
    let stdout = stdout();

    let config = Config::with_config();

    let env_file = format!(
        "{}/.config/movie_collection_rust/config.env",
        config.home_dir
    );

    if Path::new("config.env").exists() {
        dotenv::from_filename("config.env").ok();
    } else if Path::new(&env_file).exists() {
        dotenv::from_path(&env_file).ok();
    } else if Path::new("config.env").exists() {
        dotenv::from_filename("config.env").ok();
    } else {
        dotenv::dotenv().ok();
    }

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

    for f in matches
        .values_of("files")
        .ok_or_else(|| err_msg("No files given"))?
    {
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

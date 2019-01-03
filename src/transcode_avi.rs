extern crate dotenv;
extern crate failure;
extern crate movie_collection_rust;

use clap::{App, Arg};
use failure::{err_msg, Error};
use std::fs::File;
use std::io::Write;
use std::path::Path;

use movie_collection_rust::utils::{get_version_number, publish_transcode_job_to_queue, Config};

fn transcode_avi() {
    let config = Config::new().with_config();

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

    let matches = App::new("Remcom")
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

    for f in matches.values_of("files").unwrap() {
        let path = Path::new(f);

        match create_transcode_script(&config, &path) {
            Ok(s) => {
                println!("script {}", s);
                publish_transcode_job_to_queue(&s, "transcode_work_queue", "transcode_work_queue")
                    .expect("Publish to queue failed");
            }
            Err(e) => println!("error {}", e),
        }
    }
}

pub fn create_transcode_script(config: &Config, path: &Path) -> Result<String, Error> {
    let full_path = path.to_str().ok_or_else(|| err_msg("Bad path"))?;
    let fstem = path
        .file_stem()
        .ok_or_else(|| err_msg("No stem"))?
        .to_str()
        .ok_or_else(|| err_msg("failure"))?;
    let script_file = format!("{}/dvdrip/jobs/{}.sh", config.home_dir, fstem);
    if Path::new(&script_file).exists() {
        Err(err_msg("File exists"))
    } else {
        let output_file = format!("{}/dvdrip/avi/{}.mp4", config.home_dir, fstem);
        let template_file = include_str!("../templates/transcode_script.sh")
            .replace("INPUT_FILE", full_path)
            .replace("OUTPUT_FILE", &output_file)
            .replace("PREFIX", &fstem);
        let mut file = File::create(script_file.clone())?;
        file.write_all(&template_file.into_bytes())?;
        Ok(script_file)
    }
}

fn main() {
    transcode_avi();
}

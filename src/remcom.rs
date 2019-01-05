extern crate dotenv;
extern crate movie_collection_rust;

use clap::{App, Arg};
use failure::Error;
use std::path::Path;

use movie_collection_rust::config::Config;
use movie_collection_rust::utils::{
    create_move_script, create_transcode_script, get_version_number, publish_transcode_job_to_queue,
};

fn remcom() -> Result<(), Error> {
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

    let matches = App::new("Remcom")
        .version(get_version_number().as_str())
        .author("Daniel Boline <ddboline@gmail.com>")
        .about("Create script to do stuff")
        .arg(
            Arg::with_name("directory")
                .short("d")
                .long("directory")
                .value_name("DIRECTORY")
                .help("Directory"),
        )
        .arg(
            Arg::with_name("unwatched")
                .short("u")
                .long("unwatched")
                .value_name("UNWATCHED")
                .takes_value(false),
        )
        .arg(Arg::with_name("files").multiple(true))
        .get_matches();

    let unwatched = matches.is_present("unwatched");

    let directory = matches.value_of("directory").map(|d| d.to_string());

    if let Some(patterns) = matches.values_of("files") {
        let files: Vec<String> = patterns.map(|x| x.to_string()).collect();
        for file in files {
            let path = Path::new(&file);
            let ext = path.extension().unwrap().to_str().unwrap();

            if ext != "mp4" {
                match create_transcode_script(&config, &path) {
                    Ok(s) => {
                        println!("script {}", s);
                        publish_transcode_job_to_queue(
                            &s,
                            "transcode_work_queue",
                            "transcode_work_queue",
                        )
                        .expect("Publish to queue failed");
                    }
                    Err(e) => println!("error {}", e),
                }
            }

            match create_move_script(&config, directory.clone(), unwatched, &path) {
                Ok(s) => {
                    println!("script {}", s);
                    publish_transcode_job_to_queue(
                        &s,
                        "transcode_work_queue",
                        "transcode_work_queue",
                    )
                    .expect("Publish to queue failed");
                }
                Err(e) => println!("error {}", e),
            }
        }
    }
    Ok(())
}

fn main() {
    remcom().unwrap();
}

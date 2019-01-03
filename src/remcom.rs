extern crate dotenv;
extern crate movie_collection_rust;

use clap::{App, Arg};
use std::path::Path;

use movie_collection_rust::utils::{get_version_number, Config};

fn remcom() {
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

    if matches.is_present("unwatched") {
        println!("unwatched");
    }

    if let Some(directory) = matches.value_of("directory") {
        println!("directory {}", directory);
    }

    if let Some(patterns) = matches.values_of("files") {
        let strings: Vec<String> = patterns.map(|x| x.to_string()).collect();
        println!("strings {:?}", strings);
    }
}

fn main() {
    remcom();
}

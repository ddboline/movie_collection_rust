extern crate dotenv;

use clap::{App, Arg};
use std::env::var;
use std::path::Path;

fn get_version_number() -> String {
    format!(
        "{}.{}.{}{}",
        env!("CARGO_PKG_VERSION_MAJOR"),
        env!("CARGO_PKG_VERSION_MINOR"),
        env!("CARGO_PKG_VERSION_PATCH"),
        option_env!("CARGO_PKG_VERSION_PRE").unwrap_or("")
    )
}

fn remcom() {
    let home_dir = var("HOME").expect("No HOME directory...");

    let env_file = format!("{}/.config/movie_collection_rust/config.env", home_dir);

    if Path::new("config.env").exists() {
        dotenv::from_filename("config.env").ok();
    } else if Path::new(&env_file).exists() {
        dotenv::from_path(&env_file).ok();
    } else if Path::new("config.env").exists() {
        dotenv::from_filename("config.env").ok();
    } else {
        dotenv::dotenv().ok();
    }

    let suffixes = vec!["avi", "mp4", "mkv"];

    let movie_dirs: Vec<String> = var("MOVIEDIRS")
        .expect("MOVIEDIRS env variable not set")
        .split(",")
        .filter_map(|d| {
            if Path::new(d).exists() {
                Some(d.to_string())
            } else {
                None
            }
        })
        .collect();

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

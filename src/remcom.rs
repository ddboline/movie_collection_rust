extern crate dotenv;
extern crate movie_collection_rust;

use clap::{App, Arg};
use failure::Error;

use movie_collection_rust::config::Config;
use movie_collection_rust::utils::{
    create_move_script, create_transcode_script, get_version_number,
    publish_transcode_job_to_queue, remcom_single_file,
};

fn remcom() -> Result<(), Error> {
    let config = Config::with_config();

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
            remcom_single_file(&file, &directory, unwatched);
        }
    }
    Ok(())
}

fn main() {
    remcom().unwrap();
}

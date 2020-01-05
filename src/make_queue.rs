use clap::{App, Arg};
use failure::Error;

use movie_collection_lib::common::make_queue::make_queue_worker;
use movie_collection_lib::common::utils::get_version_number;

fn make_queue() -> Result<(), Error> {
    let matches = App::new("Parse IMDB")
        .version(get_version_number().as_str())
        .author("Daniel Boline <ddboline@gmail.com>")
        .about("Parse IMDB.com")
        .arg(
            Arg::with_name("add")
                .short("a")
                .long("add")
                .value_name("ADD")
                .takes_value(true)
                .multiple(true)
                .help("Add file(s)"),
        )
        .arg(
            Arg::with_name("remove")
                .short("r")
                .long("remove")
                .value_name("REMOVE")
                .takes_value(true)
                .multiple(true)
                .help("Remove entry by index OR filename"),
        )
        .arg(
            Arg::with_name("time")
                .short("t")
                .long("time")
                .takes_value(false)
                .help("Get runtime of file"),
        )
        .arg(
            Arg::with_name("shows")
                .short("s")
                .long("shows")
                .takes_value(false)
                .help("List TV Shows"),
        )
        .arg(
            Arg::with_name("patterns")
                .value_name("PATTERNS")
                .help("Patterns"),
        )
        .get_matches();
    let add_files: Option<Vec<_>> = matches
        .values_of("add")
        .map(|v| v.map(ToString::to_string).collect());
    let del_files: Option<Vec<_>> = matches
        .values_of("remove")
        .map(|v| v.map(ToString::to_string).collect());
    let do_time = matches.is_present("time");
    let patterns: Vec<_> = matches
        .values_of("patterns")
        .map_or_else(Vec::new, |v| v.map(|s| s.to_string()).collect());
    let do_shows = matches.is_present("shows");

    let patterns: Vec<_> = patterns.iter().map(String::as_str).collect();
    make_queue_worker(add_files, del_files, do_time, &patterns, do_shows)
}

fn main() {
    env_logger::init();

    match make_queue() {
        Ok(_) => (),
        Err(e) => {
            if e.to_string().contains("Broken pipe") {
            } else {
                panic!("{}", e)
            }
        }
    }
}

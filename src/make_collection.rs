use clap::{App, Arg};
use failure::Error;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use std::io;
use std::io::Write;

use movie_collection_lib::common::movie_collection::MovieCollection;
use movie_collection_lib::common::utils::{get_version_number, get_video_runtime};

fn make_collection() -> Result<(), Error> {
    let matches = App::new("Collection Query/Parser")
        .version(get_version_number().as_str())
        .author("Daniel Boline <ddboline@gmail.com>")
        .about("Query and Parse Video Collectioin")
        .arg(
            Arg::with_name("parse")
                .short("p")
                .long("parse")
                .value_name("PARSE")
                .takes_value(false)
                .help("Parse collection for new videos"),
        )
        .arg(
            Arg::with_name("time")
                .short("t")
                .long("time")
                .value_name("TIME")
                .takes_value(false)
                .help("Run time"),
        )
        .arg(
            Arg::with_name("shows")
                .value_name("SHOWS")
                .help("Shows")
                .multiple(true),
        )
        .get_matches();

    let do_parse = matches.is_present("parse");
    let do_time = matches.is_present("time");
    let shows = matches
        .values_of("shows")
        .map(|s| s.map(|x| x.to_string()).collect())
        .unwrap_or_else(Vec::new);

    let stdout = io::stdout();

    let mc = MovieCollection::new();
    if !do_parse {
        let shows = mc.search_movie_collection(&shows)?;
        if do_time {
            let shows: Result<Vec<_>, Error> = shows
                .into_par_iter()
                .map(|result| {
                    let timeval = get_video_runtime(&result.path)?;
                    Ok((timeval, result))
                })
                .collect();
            for (timeval, show) in shows? {
                writeln!(stdout.lock(), "{} {}", timeval, show)?;
            }
        } else {
            for show in shows {
                writeln!(stdout.lock(), "{}", show)?;
            }
        }
    } else {
        mc.make_collection()?;
        mc.fix_collection_show_id()?;
    }
    Ok(())
}

fn main() {
    env_logger::init();

    match make_collection() {
        Ok(_) => {}
        Err(e) => {
            if e.to_string().contains("Broken pipe") {
            } else {
                panic!("{}", e)
            }
        }
    }
}

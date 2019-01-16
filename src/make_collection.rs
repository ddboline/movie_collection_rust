extern crate failure;
extern crate movie_collection_rust;
extern crate rayon;

use clap::{App, Arg};
use failure::Error;
use rayon::prelude::*;

use movie_collection_rust::common::movie_collection::MovieCollectionDB;
use movie_collection_rust::common::utils::{get_version_number, get_video_runtime, map_result_vec};

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

    if !do_parse {
        let mq = MovieCollectionDB::new();
        let shows = mq.search_movie_collection(&shows)?;
        if do_time {
            let shows: Vec<Result<_, Error>> = shows
                .into_par_iter()
                .map(|result| {
                    let timeval = get_video_runtime(&result.path)?;
                    Ok((timeval, result))
                })
                .collect();
            let shows = map_result_vec(shows)?;
            for (timeval, show) in shows {
                println!("{} {}", timeval, show);
            }
        } else {
            for show in shows {
                println!("{}", show);
            }
        }
    } else {
        let mq = MovieCollectionDB::new();
        mq.make_collection()?;
        mq.fix_collection_show_id()?;
    }
    Ok(())
}

fn main() {
    make_collection().unwrap();
}

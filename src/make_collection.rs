extern crate failure;
extern crate movie_collection_rust;

use clap::{App, Arg};
use failure::Error;

use movie_collection_rust::config::Config;
use movie_collection_rust::movie_collection::MovieCollection;
use movie_collection_rust::utils::get_version_number;

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
                .help("Season number"),
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
        .unwrap_or_else(|| Vec::new());

    println!("shows {:?} parse {} time {}", shows, do_parse, do_time);

    if !do_parse {
        let mq = MovieCollection::new();
        let shows = mq.search_movie_collection(&shows)?;
        for show in shows {
            println!("{}", show);
        }
    } else {
        let mq = MovieCollection::new();
        mq.make_collection()?;
        mq.fix_collection_show_id()?;
    }
    Ok(())
}

fn main() {
    make_collection().unwrap();
}

extern crate failure;
extern crate movie_collection_rust;
extern crate rayon;

use clap::{App, Arg};
use failure::Error;

use movie_collection_rust::common::trakt_utils::{
    sync_trakt_with_db, trakt_app_parse, TraktActions, TraktCommands,
};
use movie_collection_rust::common::utils::get_version_number;

fn trakt_app() -> Result<(), Error> {
    let matches = App::new("Trakt Query/Parser")
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
            Arg::with_name("commands")
                .value_name("COMMANDS")
                .help("Commands: trakt-app <cal|watchlist|watched> <list|add|rm> <show> (season) (episode)")
                .multiple(true),
        )
        .get_matches();

    let do_parse = matches.is_present("parse");

    let commands: Vec<String> = matches
        .values_of("commands")
        .map(|v| v.map(|s| s.to_string()).collect())
        .unwrap_or_else(Vec::new);

    let trakt_command = match commands.get(0) {
        Some(c) => TraktCommands::from_command(c),
        None => TraktCommands::None,
    };
    let trakt_action = match commands.get(1) {
        Some(a) => TraktActions::from_command(a),
        None => TraktActions::None,
    };
    let show = commands.get(2).cloned();
    let season: i32 = match commands.get(3) {
        Some(c) => {
            if let Ok(s) = c.parse() {
                s
            } else {
                -1
            }
        }
        _ => -1,
    };
    let episode: i32 = match commands.get(4) {
        Some(c) => {
            if let Ok(s) = c.parse() {
                s
            } else {
                -1
            }
        }
        _ => -1,
    };

    if do_parse {
        sync_trakt_with_db()?;
    } else {
        trakt_app_parse(
            &trakt_command,
            &trakt_action,
            show.as_ref(),
            season,
            episode,
        )?;
    }

    Ok(())
}

fn main() {
    trakt_app().unwrap();
}

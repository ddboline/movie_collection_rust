use anyhow::Error;
use structopt::StructOpt;

use movie_collection_lib::movie_collection::MovieCollection;
use movie_collection_lib::trakt_utils::{
    sync_trakt_with_db, trakt_app_parse, TraktActions, TraktCommands,
};

#[derive(StructOpt)]
/// Query and Parse Trakt.tv
struct TraktAppOpts {
    #[structopt(long, short)]
    /// Parse collection for new videos
    parse: bool,

    /// cal, watchlist, watched
    #[structopt(parse(from_str))]
    trakt_command: Option<TraktCommands>,

    /// list, add, rm
    #[structopt(parse(from_str))]
    trakt_action: Option<TraktActions>,

    /// show
    show: Option<String>,

    /// season
    season: Option<i32>,

    /// episode
    episode: Vec<i32>,
}

async fn trakt_app() -> Result<(), Error> {
    let opts = TraktAppOpts::from_args();

    let do_parse = opts.parse;

    let trakt_command = opts.trakt_command.unwrap_or(TraktCommands::None);
    let trakt_action = opts.trakt_action.unwrap_or(TraktActions::None);
    let show = opts.show.as_deref();
    let season = opts.season.unwrap_or(-1);

    let mc = MovieCollection::new();

    if do_parse {
        sync_trakt_with_db(&mc).await
    } else {
        trakt_app_parse(&trakt_command, trakt_action, show, season, &opts.episode).await
    }
}

#[tokio::main]
async fn main() {
    env_logger::init();

    match trakt_app().await {
        Ok(_) => (),
        Err(e) => {
            if e.to_string().contains("Broken pipe") {
            } else {
                panic!("{}", e)
            }
        }
    }
}

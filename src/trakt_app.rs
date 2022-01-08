#![allow(clippy::used_underscore_binding)]

use anyhow::Error;
use stack_string::StackString;
use stdout_channel::StdoutChannel;
use structopt::StructOpt;

use movie_collection_lib::{
    config::Config,
    movie_collection::MovieCollection,
    pgpool::PgPool,
    trakt_connection::TraktConnection,
    trakt_utils::{sync_trakt_with_db, trakt_app_parse, TraktActions, TraktCommands},
};

#[derive(StructOpt)]
/// Query and Parse Trakt.tv
struct TraktAppOpts {
    #[structopt(long, short)]
    /// Parse collection for new videos
    parse: bool,

    #[structopt(long, short)]
    /// Optional imdb link
    imdb_link: Option<StackString>,

    /// cal, watchlist, watched
    #[structopt(parse(from_str))]
    trakt_command: Option<TraktCommands>,

    /// list, add, rm
    #[structopt(parse(from_str))]
    trakt_action: Option<TraktActions>,

    /// show
    show: Option<StackString>,

    /// season
    season: Option<i32>,

    /// episode
    episode: Vec<i32>,
}

async fn trakt_app() -> Result<(), Error> {
    let opts = TraktAppOpts::from_args();
    let config = Config::with_config()?;
    let do_parse = opts.parse;
    let pool = PgPool::new(&config.pgurl);
    let stdout = StdoutChannel::new();

    let trakt_command = opts.trakt_command.unwrap_or(TraktCommands::None);
    let trakt_action = opts.trakt_action.unwrap_or(TraktActions::None);
    let show = opts.show.as_ref().map(StackString::as_str);
    let imdb_link = opts.imdb_link.as_ref().map(StackString::as_str);
    let season = opts.season.unwrap_or(-1);

    let mc = MovieCollection::new(&config, &pool, &stdout);
    let trakt = TraktConnection::new(config.clone());

    let result = if do_parse {
        sync_trakt_with_db(&trakt, &mc).await
    } else {
        trakt_app_parse(
            &config,
            &trakt,
            &trakt_command,
            trakt_action,
            show,
            imdb_link,
            season,
            &opts.episode,
            &stdout,
            &pool,
        )
        .await
    };
    mc.stdout.close().await?;
    result
}

#[tokio::main]
async fn main() {
    env_logger::init();

    match trakt_app().await {
        Ok(_) => (),
        Err(e) => {
            let e = StackString::from_display(e);
            if e.contains("Broken pipe") {
            } else {
                panic!("{}", e);
            }
        }
    }
}

#![allow(clippy::must_use_candidate)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::similar_names)]
#![allow(clippy::shadow_unrelated)]
#![allow(clippy::missing_errors_doc)]

pub mod config;
pub mod imdb_episodes;
pub mod imdb_ratings;
pub mod imdb_utils;
pub mod make_list;
pub mod make_queue;
pub mod movie_collection;
pub mod movie_queue;
pub mod parse_imdb;
pub mod pgpool;
pub mod stdout_channel;
pub mod trakt_instance;
pub mod trakt_utils;
pub mod tv_show_source;
pub mod utils;

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
#![allow(clippy::struct_excessive_bools)]
#![allow(clippy::used_underscore_binding)]
#![allow(clippy::upper_case_acronyms)]
#![allow(clippy::inconsistent_struct_constructor)]
#![allow(clippy::default_trait_access)]

pub mod config;
pub mod datetime_wrapper;
pub mod imdb_episodes;
pub mod imdb_ratings;
pub mod imdb_utils;
pub mod iso_8601_datetime;
pub mod make_list;
pub mod make_queue;
pub mod movie_collection;
pub mod movie_queue;
pub mod naivedate_wrapper;
pub mod parse_imdb;
pub mod pgpool;
pub mod plex_events;
pub mod trakt_connection;
pub mod trakt_utils;
pub mod transcode_service;
pub mod tv_show_source;
pub mod utils;

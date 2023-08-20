use anyhow::{format_err, Error};
use dioxus::prelude::{
    dioxus_elements, inline_props, rsx, Element, GlobalAttributes, LazyNodes, Props, Scope,
    VirtualDom,
};
use futures::{future::try_join_all, TryStreamExt};
use rust_decimal_macros::dec;
use stack_string::{format_sstr, StackString};
use std::{
    collections::{HashMap, HashSet},
    ffi::OsStr,
    fmt::Write,
    path::Path,
    sync::Arc,
};
use stdout_channel::{MockStdout, StdoutChannel};
use time::{macros::format_description, Duration, OffsetDateTime};
use time_tz::OffsetDateTimeExt;
use uuid::Uuid;

use movie_collection_lib::{
    config::Config,
    date_time_wrapper::DateTimeWrapper,
    imdb_episodes::{ImdbEpisodes, ImdbSeason},
    imdb_ratings::ImdbRatings,
    make_list::FileLists,
    movie_collection::{MovieCollection, NewEpisodesResult, TvShowsResult},
    movie_queue::{MovieQueueDB, MovieQueueResult},
    parse_imdb::{ParseImdb, ParseImdbOptions},
    pgpool::PgPool,
    plex_events::{EventOutput, PlexSectionType},
    trakt_connection::TraktConnection,
    trakt_utils::{get_watched_shows_db, TraktCalEntry, WatchListMap},
    transcode_service::{
        movie_directories, ProcInfo, ProcStatus, TranscodeServiceRequest, TranscodeStatus,
    },
    tv_show_source::TvShowSource,
    utils::parse_file_stem,
};

use crate::{
    movie_queue_routes::{ProcessShowItem, TvShowsMap},
    OrderBy,
};

pub fn index_body() -> String {
    let mut app = VirtualDom::new(index_element);
    drop(app.rebuild());
    dioxus_ssr::render(&app)
}

fn index_element(cx: Scope) -> Element {
    cx.render(rsx! {
        head {
            style {
                include_str!("../../templates/style.css")
            }
        },
        body {
            script {src: "/list/scripts.js"},
            h3 {
                "align": "center",
                input {
                    "type": "button",
                    name: "tvshows",
                    value: "TVShows",
                    "onclick": "updateMainArticle('/list/tvshows');",
                },
                input {
                    "type": "button",
                    "name": "list_cal",
                    value: "LocalCalendar",
                    "onclick": "updateMainArticle('/list/cal?source=all');"
                },
                input {
                    "type": "button",
                    "name": "watchlist",
                    value: "WatchList",
                    "onclick": "updateMainArticle('/trakt/watchlist');"
                },
                input {
                    "type": "button",
                    "name": "trakt_cal",
                    value: "TraktCalendar",
                    "onclick": "updateMainArticle('/trakt/cal');"
                },
                input {
                    "type": "button",
                    "name": "list",
                    value: "FullQueue",
                    "onclick": "updateMainArticle('/list/full_queue?limit=20');"
                },
                input {
                    "type": "button",
                    "name": "plex",
                    value: "PlexList",
                    "onclick": "updateMainArticle('/list/plex?limit=20');"
                },
                input {
                    "type": "button",
                    "name": "transocde_status",
                    value: "TranscodeStatus",
                    "onclick": "get_transcode_status();"
                },
                input {
                    "type": "button",
                    "name": "refresh",
                    value: "RefreshAuth",
                    "onclick": "refreshAuth();"
                },
                input {
                    "type": "button",
                    "name": "auth",
                    value: "Auth",
                    "onclick": "traktAuth();"
                },
            },
            h3 {
                "align": "center",
                article {
                    id: "main_article",
                    "align": "center",
                }
            }
        }
    })
}

#[derive(PartialEq)]
struct QueueEntry {
    row: MovieQueueResult,
    ext: StackString,
    file_name: StackString,
    file_stem: StackString,
    collection_idx: Option<Uuid>,
    metadata_key: Option<StackString>,
}

/// # Errors
/// Returns error if db query fails
pub async fn movie_queue_body(
    config: &Config,
    pool: &PgPool,
    patterns: Vec<StackString>,
    queue: Vec<MovieQueueResult>,
    search: Option<StackString>,
    offset: Option<u64>,
    limit: Option<u64>,
    order_by: Option<OrderBy>,
) -> Result<String, Error> {
    let mock_stdout = MockStdout::new();
    let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

    let mc = Arc::new(MovieCollection::new(config, pool, &stdout));

    let futures = queue.into_iter().map(|row| {
        let mc = mc.clone();
        async move {
            let path = Path::new(row.path.as_str());
            let ext = path
                .extension()
                .ok_or_else(|| format_err!("Cannot determine extension"))?
                .to_string_lossy()
                .into();
            let file_name = path
                .file_name()
                .ok_or_else(|| format_err!("Invalid path"))?
                .to_string_lossy()
                .into();
            let file_stem = path
                .file_stem()
                .ok_or_else(|| format_err!("Invalid path"))?
                .to_string_lossy()
                .into();

            let mut collection_idx = None;
            let mut metadata_key = None;

            if ext == "mp4" {
                let idx = mc
                    .get_collection_index(&row.path)
                    .await?
                    .unwrap_or_else(Uuid::new_v4);
                metadata_key = mc.get_plex_metadata_key(idx).await?;
                collection_idx.replace(idx);
            }

            Ok(QueueEntry {
                row,
                ext,
                file_name,
                file_stem,
                collection_idx,
                metadata_key,
            })
        }
    });
    let entries: Result<Vec<QueueEntry>, Error> = try_join_all(futures).await;
    let entries = entries?;
    let mut app = VirtualDom::new_with_props(
        movie_queue_element,
        movie_queue_elementProps {
            config: config.clone(),
            patterns,
            entries,
            search,
            offset,
            limit,
            order_by,
        },
    );
    drop(app.rebuild());
    Ok(dioxus_ssr::render(&app))
}

#[inline_props]
fn movie_queue_element(
    cx: Scope,
    config: Config,
    patterns: Vec<StackString>,
    entries: Vec<QueueEntry>,
    search: Option<StackString>,
    offset: Option<u64>,
    limit: Option<u64>,
    order_by: Option<OrderBy>,
) -> Element {
    let watchlist_url = if patterns.is_empty() {
        format_sstr!("/trakt/watchlist")
    } else {
        let patterns = patterns.join("_");
        format_sstr!("/trakt/watched/list/{patterns}")
    };
    let search_str = search
        .as_ref()
        .map_or_else(StackString::new, |s| format_sstr!("&q={s}"));
    let search_str = search_str.as_str();

    let queue_entries = entries.iter().enumerate().map(|(idx, entry)| {
        let (_, season, episode) = parse_file_stem(&entry.file_stem);
        let host = config.plex_host.as_ref();
        let server = config.plex_server.as_ref();
        let file_name = &entry.file_name;
        let filename_entry = if &entry.ext == "mp4" {
            let index = entry.row.idx;
            if let (Some(metadata_key), Some(host), Some(server)) = (&entry.metadata_key, host, server) {
                rsx! {
                    a {
                        href: "http://{host}:32400/web/index.html#!/server/{server}/details?key={metadata_key}",
                        target: "_blank",
                        "{index} {file_name}",
                    }
                }
            } else if let Some(collection_idx) = entry.collection_idx {
                let play_url = format_sstr!("/list/play/{collection_idx}");
                rsx! {
                    a {
                        href: "javascript:updateMainArticle('{play_url}');",
                        "{index} {file_name}",
                    }
                }
            } else {
                rsx! {
                    "{index} {file_name}",
                }
            }
        } else {
            rsx! {"{file_name}"}
        };

        let row_entry = if let Some(link) = entry.row.link.as_ref() {
            rsx! {
                td {
                    filename_entry,
                },
                td {
                    a {
                        href: "https://www.imdb.com/title/{link}",
                        target: "_blank",
                        "imdb",
                    }
                }
            }
        } else {
            rsx! {
                td {filename_entry},
            }
        };

        let remove_button = rsx! {
            button {
                "type": "submit",
                id: "{file_name}",
                "onclick": "delete_show('{file_name}');",
                "remove",
            }
        };

        let transcode_button = if entry.ext == "mp4" {
            None
        } else if season != -1 && episode != -1 {
            Some(rsx! {
                button {
                    "type": "submit",
                    id: "{file_name}",
                    "onclick": "transcode_queue('{file_name}');",
                    "transcode",
                }
            })
        } else {
            let entries: Vec<_> = entry.row.path.split('/').collect();
            let len_entries = entries.len();
            let directory = entries[len_entries - 2];
            Some(rsx! {
                button {
                    "type": "submit",
                    id: "{file_name}",
                    "onclick": "transcode_queue_directory('{file_name}', '{directory}');",
                    "transcode",
                }
            })
        };

        rsx! {
            tr {
                key: "queue-key-{idx}",
                row_entry,
                td {
                    remove_button,
                },
                td {
                    transcode_button,
                }
            }
        }
    });
    let order_by = order_by.unwrap_or(OrderBy::Desc);
    let limit = limit.unwrap_or(20);
    let previous_button = if let Some(offset) = offset {
        if *offset < limit {
            None
        } else {
            let new_offset = *offset - limit;
            Some(rsx! {
                button {
                    "type": "submit",
                    name: "previous",
                    value: "Previous",
                    "onclick": "updateMainArticle('/list/full_queue?limit={limit}&offset={new_offset}&order_by={order_by}{search_str}')",
                    "Previous",
                }
            })
        }
    } else {
        None
    };
    let offset = offset.unwrap_or(0);
    let new_offset = offset + limit;
    let next_button = rsx! {
        button {
            "type": "submit",
            name: "next",
            value: "Next",
            "onclick": "updateMainArticle('/list/full_queue?limit={limit}&offset={new_offset}&order_by={order_by}{search_str}')",
            "Next",
        }
    };
    let order_by_button = {
        let order_by = match order_by {
            OrderBy::Asc => OrderBy::Desc,
            OrderBy::Desc => OrderBy::Asc,
        };
        rsx! {
            button {
                "type": "submit",
                name: "{order_by}",
                value: "{order_by}",
                "onclick": "updateMainArticle('/list/full_queue?limit={limit}&offset={offset}&order_by={order_by}{search_str}')",
                "{order_by}",
            }
        }
    };
    let search = rsx! {
        form {
            input {
                "type": "text",
                name: "search",
                id: "full_queue_search",
            },
            input {
                "type": "button",
                name: "submitSearch",
                value: "Search",
                "onclick": "searchFullQueue({offset}, '{order_by}')",
            }
        }
    };

    cx.render(rsx! {
        br {
            a {
                href: "javascript:updateMainArticle('/list/tvshows')",
                "Go Back",
            }
        },
        a {
            href: "javascript:updateMainArticle('{watchlist_url}')",
            "Watch List",
        },
        br {
            order_by_button,
            previous_button,
            next_button,
        }
        search,
        table {
            "border": "0",
            "align": "center",
            tbody {
                queue_entries
            }
        }
    })
}

/// # Errors
/// Returns error if db query fails
pub fn play_worker_body(
    config: &Config,
    full_path: &Path,
    last_url: Option<StackString>,
) -> Result<String, Error> {
    let file_name: StackString = full_path
        .file_name()
        .ok_or_else(|| format_err!("Invalid path"))?
        .to_string_lossy()
        .into();
    if let Some(partial_path) = &config.video_playback_path {
        let partial_path = partial_path.join("videos").join("partial");
        let partial_path = partial_path.join(file_name.as_str());
        if partial_path.exists() {
            std::fs::remove_file(&partial_path)?;
        }
        #[cfg(target_family = "unix")]
        std::os::unix::fs::symlink(full_path, &partial_path).map_err(Into::<Error>::into)?;

        let mut app = VirtualDom::new_with_props(
            play_worker_element,
            play_worker_elementProps {
                file_name,
                last_url,
            },
        );
        drop(app.rebuild());
        Ok(dioxus_ssr::render(&app))
    } else {
        Err(format_err!("video playback path does not exist"))
    }
}

#[inline_props]
fn play_worker_element(
    cx: Scope,
    file_name: StackString,
    last_url: Option<StackString>,
) -> Element {
    let back_button = if let Some(last_url) = last_url {
        Some(rsx! {
            input {
                "type": "button",
                name: "back",
                value: "Back",
                "onclick": "updateMainArticle('{last_url}');",
            }
        })
    } else {
        None
    };
    let url = format_sstr!("/videos/partial/{file_name}");

    cx.render(rsx! {
        br {
            back_button,
        }
        "{file_name}",
        video {
            width: "720",
            controls: "true",
            source {
                src: "{url}",
                "type": "video/mp4",
            },
            "Your browser does not support HTML5 video.",
        }
    })
}

type QueueKey = (StackString, i32, i32);
type QueueValue = (
    Uuid,
    Option<StackString>,
    Option<StackString>,
    Option<StackString>,
);

/// # Errors
/// Returns error if db queries fail
pub async fn find_new_episodes_body(
    config: &Config,
    pool: &PgPool,
    stdout: &StdoutChannel<StackString>,
    shows: Option<StackString>,
    source: Option<TvShowSource>,
) -> Result<String, Error> {
    let mc = MovieCollection::new(config, pool, stdout);
    let shows_filter: Option<HashSet<StackString>> =
        shows.map(|s| s.split(',').map(Into::into).collect());

    let local = DateTimeWrapper::local_tz();
    let mindate = (OffsetDateTime::now_utc() + Duration::days(-14))
        .to_timezone(local)
        .date();
    let maxdate = (OffsetDateTime::now_utc() + Duration::days(7))
        .to_timezone(local)
        .date();

    let mq = MovieQueueDB::new(config, pool, stdout);

    let episodes = mc.get_new_episodes(mindate, maxdate, source).await?;

    let shows: HashSet<StackString> = episodes
        .iter()
        .filter_map(|s| {
            let show = s.show.clone();
            match shows_filter.as_ref() {
                Some(f) => {
                    if f.contains(show.as_str()) {
                        Some(show)
                    } else {
                        None
                    }
                }
                None => Some(show),
            }
        })
        .collect();

    let mut queue = Vec::new();

    for show in shows {
        let movie_queue = mq.print_movie_queue(&[&show], None, None, None).await?;
        for s in movie_queue {
            if let Some(u) = mc.get_collection_index(&s.path).await? {
                let metadata_key = mc.get_plex_metadata_key(u).await?;
                let host = config.plex_host.clone();
                let server = config.plex_server.clone();
                queue.push((
                    (
                        s.show.clone().unwrap_or_else(|| "".into()),
                        s.season.unwrap_or(-1),
                        s.episode.unwrap_or(-1),
                    ),
                    (u, metadata_key, host, server),
                ));
            }
        }
    }

    let queue: HashMap<QueueKey, QueueValue> = queue.into_iter().collect();

    let mut app = VirtualDom::new_with_props(
        find_new_episodes_element,
        find_new_episodes_elementProps {
            episodes,
            queue,
            source,
        },
    );
    drop(app.rebuild());
    Ok(dioxus_ssr::render(&app))
}

#[inline_props]
fn find_new_episodes_element(
    cx: Scope,
    episodes: Vec<NewEpisodesResult>,
    queue: HashMap<QueueKey, QueueValue>,
    source: Option<TvShowSource>,
) -> Element {
    let episode_entries = episodes.iter().enumerate().map(|(idx, epi)| {
        let season = epi.season;
        let link = &epi.link;
        let title = &epi.title;
        let epurl = &epi.epurl;
        let episode = epi.episode;
        let key = (epi.show.clone(), epi.season, episode);
        let airdate = epi.airdate;
        let eptitle = &epi.eptitle;
        let eprating = epi.eprating.unwrap_or(-1.0);
        let rating = epi.rating;
        let show = &epi.show;

        let title_element = match queue.get(&key) {
            Some((_, Some(metadata_key), Some(host), Some(server))) => {
                rsx! {
                    a {
                        href: "http://{host}:32400/web/index.html#!/server/{server}/details?key={metadata_key}",
                        target: "_blank",
                        "{eptitle}"
                    }
                }
            },
            Some((idx, _, _, _)) => {
                rsx! {
                    a {
                        href: "javascript:updateMainArticle('/list/play/{idx}');",
                        "{eptitle}",
                    }
                }
            },
            None => {rsx! {"{eptitle}"}},
        };
        let mut cal_url = format_sstr!("/list/cal");
        if let Some(s) = source.as_ref() {
            write!(cal_url, "?source={s}").unwrap();
        }

        rsx! {
            tr {
                key: "episode-key-{idx}",
                td {
                    a {
                        href: "javascript:updateMainArticle('/trakt/watched/list/{link}/{season}')",
                        "{title}",
                    }
                },
                td {
                    title_element,
                },
                td {
                    a {
                        href: "https://www.imdb.com/title/{epurl}",
                        target: "_blank",
                        "s{season:02} ep{episode:02}",
                    }
                },
                td {
                    "raging: {eprating:0.1} / {rating:0.1}"
                },
                td {
                    "{airdate}",
                },
                td {
                    button {
                        "type": "submit",
                        id: "update-database-{show}-{link}-{season}",
                        "onclick": "imdb_update('{show}', '{link}', {season}, '{cal_url}')",
                        "update database",
                    }
                }
            }
        }
    });

    cx.render(rsx! {
        br {
            a {
                href: "javascript:updateMainArticle('/list/tvshows')",
                "Go Back",
            },
        },
        input {
            "type": "button",
            name: "list_cal",
            value: "TVCalendar",
            "onclick": "updateMainArticle('/list/cal');"
        },
        input {
            "type": "button",
            name: "list_cal",
            value: "NetflixCalendar",
            "onclick": "updateMainArticle('/list/cal?source=netflix');"
        },
        input {
            "type": "button",
            name: "list_cal",
            value: "AmazonCalendar",
            "onclick": "updateMainArticle('/list/cal?source=amazon');"
        },
        input {
            "type": "button",
            name: "list_cal",
            value: "HuluCalendar",
            "onclick": "updateMainArticle('/list/cal?source=hulu');"
        },
        "<br>",
        button {
            name: "remcomout",
            id: "remcomoutput",
            "&nbsp;"
        },
        table {
            "border": "0",
            "align": "center",
            episode_entries,
        }
    })
}

pub fn tvshows_body(show_map: TvShowsMap, tvshows: Vec<TvShowsResult>) -> String {
    let tvshows: HashSet<_> = tvshows
        .into_iter()
        .map(|s| {
            let item: ProcessShowItem = s.into();
            item
        })
        .collect();
    let watchlist: HashSet<_> = show_map
        .into_iter()
        .map(|(link, (show, s, source))| {
            let item = ProcessShowItem {
                show,
                title: s.title,
                link: s.link,
                source,
            };
            debug_assert!(link.as_str() == item.link.as_str());
            item
        })
        .collect();

    let mut app =
        VirtualDom::new_with_props(tvshows_element, tvshows_elementProps { tvshows, watchlist });
    drop(app.rebuild());
    dioxus_ssr::render(&app)
}

#[inline_props]
fn tvshows_element(
    cx: Scope,
    tvshows: HashSet<ProcessShowItem>,
    watchlist: HashSet<ProcessShowItem>,
) -> Element {
    let watchlist_shows = watchlist
        .iter()
        .filter(|item| tvshows.get(item.link.as_str()).is_none());

    let mut shows: Vec<_> = tvshows.iter().chain(watchlist_shows).collect();
    shows.sort_by(|x, y| x.show.cmp(&y.show));

    let entries = shows.into_iter().enumerate().map(|(idx, item)| {
        let link = item.link.as_str();
        let title = &item.title;
        let show = &item.show;
        let has_watchlist = watchlist.contains(link);
        let watchlist = if has_watchlist {
            Some(rsx! {
                a {
                    href: "javascript:updateMainArticle('/trakt/watched/list/{link}')",
                    "watchlist",
                }
            })
        } else {
            None
        };
        let title_element = if tvshows.contains(link) {
            rsx! {
                a {
                    href: "javascript:updateMainArticle('/list/queue/{show}')",
                    "{title}",
                }
            }
        } else {
            rsx! {
                a {
                    href: "javascript:updateMainArticle('/trakt/watched/list/{link}",
                    "{title}",
                }
            }
        };
        let src = match item.source {
            Some(TvShowSource::Netflix) => {
                Some(rsx! {a {href: "https://netflix.com", target: "_blank", "netflix"}})
            }
            Some(TvShowSource::Hulu) => {
                Some(rsx! {a {href: "https://hulu.com", target: "_blank", "hulu"}})
            }
            Some(TvShowSource::Amazon) => {
                Some(rsx! {a {href: "https://amazon.com", target: "_blank", "amazon"}})
            }
            _ => None,
        };
        let sh = if has_watchlist {
            rsx! {
                td {
                    button {
                        "type": "submit",
                        id: "remove-link",
                        "onclick": "watchlist_rm('{link}');",
                        "remove to watchlist"
                    }
                }
            }
        } else {
            rsx! {
                td {
                    button {
                        "type": "submit",
                        id: "add-link",
                        "onclick": "watchlist_add('{link}');",
                        "add to watchlist"
                    }
                }
            }
        };

        rsx! {
            tr {
                key: "show-key-{idx}",
                td {title_element},
                td {
                    a {
                        href: "https://www.imdb.com/title/{link}",
                        target: "_blank",
                        "imdb",
                    }
                },
                td {src},
                td {watchlist},
                td {sh},
            }
        }
    });

    cx.render(rsx! {
        br {
            a {
                href: "javascript:updateMainArticle('/list/watchlist')",
                "Go Back",
            },
        }
        a {
            href: "javascript:updateMainArticle('/trakt/watchlist')",
            "Watch List",
        },
        "<br>",
        button {
            name: "remcomout",
            id: "remcomoutput",
            "&nbsp;",
        },
        "<br>",
        table {
            "border": "0",
            "align": "center",
            entries,
        }
    })
}

pub fn watchlist_body(shows: WatchListMap) -> String {
    let mut shows: Vec<_> = shows
        .into_iter()
        .map(|(_, (_, s, source))| WatchListEntry {
            title: s.title,
            link: s.link,
            source,
        })
        .collect();
    shows.sort();
    let mut app = VirtualDom::new_with_props(watchlist_element, watchlist_elementProps { shows });
    drop(app.rebuild());
    dioxus_ssr::render(&app)
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct WatchListEntry {
    title: StackString,
    link: StackString,
    source: Option<TvShowSource>,
}

#[inline_props]
fn watchlist_element(cx: Scope, shows: Vec<WatchListEntry>) -> Element {
    let shows = shows.iter().enumerate().map(|(idx, entry)| {
        let title = &entry.title;
        let link = &entry.link;
        let source = entry.source;
        let options = [
            (TvShowSource::All, "all", ""),
            (TvShowSource::Amazon, "amazon", "Amazon"),
            (TvShowSource::Hulu, "hulu", "Hulu"),
            (TvShowSource::Netflix, "netflix", "Netflix"),
        ];

        rsx! {
            tr {
                key: "shows-key-{idx}",
                td {
                    a {
                        href: "javascript:updateMainArticle('/trakt/watched/list/{link}')",
                        "{title}",
                    }
                },
                td {
                    a {
                        href: "https://www.imdb.com/title/{link}",
                        target: "_blank",
                        "imdb",
                    }
                },
                td {
                    form {
                        action: "javascript:setSource('{link}', '{link}_source_id')",
                        select {
                            id: "{link}_source_id",
                            "onchange": "setSource('{link}', '{link}_source_id');",
                            options.iter().enumerate().map(|(i, (s, v, l))| {
                                if (source.is_none() && *s == TvShowSource::All) || source == Some(*s) {
                                    rsx! {
                                        option {
                                            key: "source-option-{i}",
                                            value: "{v}",
                                            selected: true,
                                            "{l}",
                                        }
                                    }
                                } else {
                                    rsx! {
                                        option {
                                            key: "source-option-{i}",
                                            value: "{v}",
                                            "{l}",
                                        }
                                    }
                                }
                            }),
                        }
                    }
                }
            }
        }
    });
    cx.render(rsx! {
        br {
            a {
                href: "javascript:updateMainArticle('/list/tvshows')",
                "Go Back",
            },
        }
        table {
            "border": "0",
            "align": "center",
            shows
        }
    })
}

pub fn trakt_watched_seasons_body(
    link: StackString,
    imdb_url: StackString,
    entries: Vec<ImdbSeason>,
) -> String {
    let mut app = VirtualDom::new_with_props(
        trakt_watched_seasons_element,
        trakt_watched_seasons_elementProps {
            link,
            imdb_url,
            entries,
        },
    );
    drop(app.rebuild());
    dioxus_ssr::render(&app)
}

#[inline_props]
fn trakt_watched_seasons_element(
    cx: Scope,
    link: StackString,
    imdb_url: StackString,
    entries: Vec<ImdbSeason>,
) -> Element {
    let entries = entries.iter().enumerate().map(|(idx, s)| {
        let show = &s.show;
        let season = s.season;
        let title = &s.title;
        let id = format_sstr!("watched_seasons_id_{show}_{link}_{season}");
        let button_add = rsx! {
            td {
                button {
                    "type": "submit",
                    id: "{id}",
                    "onclick": "imdb_update('{show}', '{link}', {season}, '/trakt/watched/list/{link}');",
                    "update database",
                }
            }
        };
        let nepisodes = s.nepisodes;

        rsx! {
            tr {
                key: "entry-key-{idx}",
                td {
                    a {
                        href: "javascript:updateMainArticle('/trakt/watched/list/{imdb_url}/{season}')",
                        "{title}",
                    }
                },
                td {"{season}"},
                td {"{nepisodes}"},
                td {button_add},
            }
        }
    });

    cx.render(rsx! {
        br {
            a {
                href: "javascript:updateMainArticle('/trakt/watchlist')",
                "Go Back",
            },
        }
        table {
            "border": "0",
            "align": "center",
            entries,
        }
    })
}

#[derive(PartialEq)]
struct CalEntry {
    cal_entry: TraktCalEntry,
    episode: Option<ImdbEpisodes>,
}

/// # Errors
/// Returns error if db query fails
pub async fn trakt_cal_http_body(pool: &PgPool, trakt: &TraktConnection) -> Result<String, Error> {
    trakt.init().await;
    let cal_list = trakt.get_calendar().await?;
    let mut entries = Vec::new();
    for cal in cal_list {
        let show = ImdbRatings::get_show_by_link(&cal.link, pool)
            .await?
            .map(|s| s.show);
        let episode = if let Some(show) = show {
            let epi = ImdbEpisodes {
                show: show.clone(),
                season: cal.season,
                episode: cal.episode,
                ..ImdbEpisodes::default()
            };
            match epi.get_index(pool).await? {
                Some(idx) => ImdbEpisodes::from_index(idx, pool).await?,
                None => None,
            }
        } else {
            None
        };
        entries.push(CalEntry {
            cal_entry: cal,
            episode,
        });
    }
    let mut app = VirtualDom::new_with_props(
        trakt_cal_http_element,
        trakt_cal_http_elementProps { entries },
    );
    drop(app.rebuild());
    Ok(dioxus_ssr::render(&app))
}

#[inline_props]
fn trakt_cal_http_element(cx: Scope, entries: Vec<CalEntry>) -> Element {
    let entries = entries.iter().enumerate().map(|(idx, entry)| {
        let link = &entry.cal_entry.link;
        let show = &entry.cal_entry.show;
        let season = entry.cal_entry.season;
        let episode = entry.cal_entry.episode;
        let airdate = entry.cal_entry.airdate;
        let button = if entry.episode.is_none() {
            Some(rsx! {
                button {
                    "type": "submit",
                    id: "entry-id-{idx}",
                    "onclick": "imdb_update('{show}', '{link}', {season}, '/trakt/cal')",
                    "update database",
                }
            })
        } else {
            None
        };
        let season_episode = if let Some(link) = &entry.cal_entry.ep_link {
            rsx! {
                a {
                    href: "https://www.imdb.com/title/{link}",
                    target: "_blank",
                    "{season} {episode}",
                }
            }
        } else if let Some(episode) = entry.episode.as_ref() {
            let link = &episode.epurl;
            let episode = episode.episode;
            rsx! {
                a {
                    href: "https://www.imdb.com/title/{link}",
                    target: "_blank",
                    "{season} {episode}",
                }
            }
        } else {
            rsx! {
                "{season} {episode}",
            }
        };
        rsx! {
            tr {
                key: "trakt-cal-key-{idx}",
                td {
                    a {
                        href: "javascript:updateMainArticle('/trakt/watched/list/{link}/{season}')",
                        "{show}",
                    }
                },
                td {
                    a {
                        href: "https://www.imdb.com/title/{link}",
                        target: "_blank",
                    }
                },
                td {
                    season_episode,
                },
                td {"{airdate}"},
                td {button},
            }
        }
    });

    cx.render(rsx! {
        br {
            a {
                href: "javascript:updateMainArticle('/list/tvshows')",
                "Go Back",
            },
        },
        table {
            "border": "0",
            "align": "center",
            entries,
        }
    })
}

/// # Errors
/// Returns error if db query fails
pub async fn watch_list_http_body(
    config: &Config,
    pool: &PgPool,
    stdout: &StdoutChannel<StackString>,
    imdb_url: &str,
    season: i32,
) -> Result<String, Error> {
    let mc = MovieCollection::new(config, pool, stdout);
    let mq = MovieQueueDB::new(config, pool, stdout);

    let show = ImdbRatings::get_show_by_link(imdb_url, pool)
        .await?
        .ok_or_else(|| format_err!("Show Doesn't exist"))?;

    let show_str = &show.show;
    let watched_episodes_db: HashSet<i32> = get_watched_shows_db(pool, show_str, Some(season))
        .await?
        .map_ok(|s| s.episode)
        .try_collect()
        .await?;

    let queue: HashMap<(StackString, i32, i32), _> = mq
        .print_movie_queue(&[show_str.as_str()], None, None, None)
        .await?
        .into_iter()
        .filter_map(|s| match &s.show {
            Some(show) => match s.season {
                Some(season) => match s.episode {
                    Some(episode) => Some(((show.clone(), season, episode), s)),
                    None => None,
                },
                None => None,
            },
            None => None,
        })
        .collect();

    let entries = mc.print_imdb_episodes(show_str, Some(season)).await?;
    let mut collection_idx_map = HashMap::new();
    let mut collection_metadata_map = HashMap::new();
    for r in &entries {
        if let Some(row) = queue.get(&(show_str.clone(), season, r.episode)) {
            if let Some(index) = mc.get_collection_index(&row.path).await? {
                collection_idx_map.insert(r.episode, index);
                if let Some(metadata_key) = mc.get_plex_metadata_key(index).await? {
                    collection_metadata_map.insert(index, metadata_key);
                }
            }
        }
    }

    let mut app = VirtualDom::new_with_props(
        watch_list_http_element,
        watch_list_http_elementProps {
            config: config.clone(),
            imdb_url: imdb_url.into(),
            show,
            season,
            entries,
            collection_idx_map,
            collection_metadata_map,
            watched_episodes_db,
        },
    );
    drop(app.rebuild());
    Ok(dioxus_ssr::render(&app))
}

#[inline_props]
fn watch_list_http_element(
    cx: Scope,
    config: Config,
    imdb_url: StackString,
    show: ImdbRatings,
    season: i32,
    entries: Vec<ImdbEpisodes>,
    collection_idx_map: HashMap<i32, Uuid>,
    collection_metadata_map: HashMap<Uuid, StackString>,
    watched_episodes_db: HashSet<i32>,
) -> Element {
    let show_str = &show.show;
    let link = &show.link;

    let entries = entries.iter().enumerate().map(|(idx, s)| {
        let eptitle = &s.eptitle;
        let play_entry = if let Some(collection_idx) = collection_idx_map.get(&s.episode) {
            let host = config.plex_host.as_ref();
            let server = config.plex_server.as_ref();
            if let (Some(metadata_key), Some(host), Some(server)) = (collection_metadata_map.get(collection_idx), host, server) {
                rsx! {
                    a {
                        href: "http://{host}:32400/web/index.html#!/server/{server}/details?key={metadata_key}",
                        target: "_blank",
                        "{eptitle}",
                    }
                }
            } else {
                rsx! {
                    a {
                        href: "javascript:updateMainArticle('/list/play/{collection_idx}');",
                        "{eptitle}",
                    }
                }
            }
        } else {
            rsx! {"{eptitle}"}
        };
        let airdate = s.airdate.map_or_else(StackString::new, StackString::from_display);
        let show_rating = show.rating.as_ref().unwrap_or(&-1.0);
        let ep_rating = s.rating.unwrap_or_else(|| dec!(-1));
        let ep_url = &s.epurl;
        let episode = s.episode;

        let button = if watched_episodes_db.contains(&s.episode) {
            rsx! {
                button {
                    "type": "submit",
                    id: "rm-watched",
                    "onclick": "watched_rm('{link}', {season}, {episode});",
                    "remove from watched",
                }
            }
        } else {
            rsx! {
                button {
                    "type": "submit",
                    id: "add-watched",
                    "onclick": "watched_add('{link}', {season}, {episode});",
                    "add to watched",
                }
            }
        };

        rsx! {
            tr {
                key: "watch-list-key-{idx}",
                td {
                    "{show_str}",
                },
                td {play_entry},
                td {
                    a {
                        href: "https://www.imdb.com/title/{ep_url}",
                        target: "_blank",
                        "s{season} ep{episode}",
                    }
                },
                td {"rating: {ep_rating:0.1} {show_rating:0.1}"},
                td {"{airdate}"},
                td {
                    button
                },
            }
        }
    });

    cx.render(rsx! {
        br {
            a {
                href: "javascript:updateMainArticle('/trakt/watched/list/{imdb_url}')",
                "Go Back",
            }
        },
        button {
            name: "remcomout",
            id: "remcomoutput",
            "&nbsp;",
        },
        button {
            "type": "submit",
            id: "watch-list-update",
            "onclick": "imdb_update('{show_str}', '{link}', {season}, '/trakt/watched/list/{link}/{season}');",
            "update database",
        },
        table {
            "border": "0",
            "align": "center",
            entries,
        }
    })
}

/// # Errors
/// Returns error if db query fails
pub async fn parse_imdb_http_body(
    imdb: &ParseImdb,
    opts: &ParseImdbOptions,
    watchlist: WatchListMap,
) -> Result<String, Error> {
    let imdb_urls = imdb.parse_imdb_worker(opts).await?;

    let mut app = VirtualDom::new_with_props(
        parse_imdb_http_element,
        parse_imdb_http_elementProps {
            imdb_urls,
            watchlist,
        },
    );
    drop(app.rebuild());
    Ok(dioxus_ssr::render(&app))
}

#[inline_props]
fn parse_imdb_http_element(
    cx: Scope,
    imdb_urls: Vec<Vec<StackString>>,
    watchlist: WatchListMap,
) -> Element {
    let entries = imdb_urls.iter().enumerate().map(move |(idx, line)| {
        let mut imdb_url = None;
        for u in line {
            if u.starts_with("tt") {
                imdb_url.replace(u.clone());
            }
        }
        let tmp = line.iter().map(|imdb_url_| {
            if imdb_url_.starts_with("tt") {
                rsx! {
                    a {
                        href: "https://www.imdb.com/title/{imdb_url_}",
                        target: "_blank",
                    }
                }
            } else {
                rsx! {"{imdb_url_}"}
            }
        });
        let button = imdb_url.map(|imdb_url| {
            if watchlist.contains_key(&imdb_url) {
                rsx! {
                    td {
                        button {
                            "type": "submit",
                            id: "watchlist-rm-{idx}",
                            "onclick": "watchlist_rm('{imdb_url}');",
                            "remove from watchlist",
                        }
                    }
                }
            } else {
                rsx! {
                    td {
                        button {
                            "type": "submit",
                            id: "watchlist-add-{idx}",
                            "onclick": "watchlist_add('{imdb_url}');",
                            "add from watchlist",
                        }
                    }
                }
            }
        });

        rsx! {
            tr {
                key: "imdb-entries-{idx}",
                td {tmp},
                td {button},
            }
        }
    });

    cx.render(rsx! {
        entries
    })
}

pub fn plex_body(
    config: Config,
    events: Vec<EventOutput>,
    section: Option<PlexSectionType>,
    offset: Option<u64>,
    limit: Option<u64>,
) -> String {
    let mut app = VirtualDom::new_with_props(
        plex_element,
        plex_elementProps {
            config,
            events,
            section,
            offset,
            limit,
        },
    );
    drop(app.rebuild());
    dioxus_ssr::render(&app)
}

#[inline_props]
fn plex_element(
    cx: Scope,
    config: Config,
    events: Vec<EventOutput>,
    section: Option<PlexSectionType>,
    offset: Option<u64>,
    limit: Option<u64>,
) -> Element {
    let local = DateTimeWrapper::local_tz();
    let entries = events.iter().enumerate().map(|(idx, event)| {
        let id = event.id;
        let last_modified = match config.default_time_zone {
            Some(tz) => {
                let tz = tz.into();
                event.last_modified.to_timezone(tz)
            }
            None => event.last_modified.to_timezone(local),
        };
        let last_modified = last_modified
            .format(format_description!(
                "[year]-[month]-[day]T[hour]:[minute]:[second][offset_hour]:[offset_minute]"
            ))
            .unwrap_or_default();
        let event_str = &event.event;
        let title = &event.title;
        let title_len = if title.len() < 10 { title.len() } else { 10 };
        let title = String::from_utf8_lossy(&title.as_bytes()[0..title_len]);
        let metadata_type = event.metadata_type.as_ref().map_or("", StackString::as_str);
        let section_title = event.section_title.as_ref().map_or("", StackString::as_str);
        let parent_title = event.parent_title.as_ref().map_or("", StackString::as_str);
        let grandparent_title = event
            .grandparent_title
            .as_ref()
            .map_or("", StackString::as_str);
        let filename = event.filename.as_ref().map_or("", StackString::as_str);
        let filestem = filename.split('/').last().unwrap_or("");
        rsx! {
            tr {
                key: "plex-event-key-{idx}",
                "style": "text-align; center;",
                td {
                    a {
                        href: "javascript:updateMainArticle('/list/plex/{id}')",
                        "{id}"
                    },
                },
                td {"{last_modified}"},
                td {"{event_str}"},
                td {"{metadata_type}"},
                td {"{section_title}"},
                td {"{filestem} {grandparent_title} {parent_title} {title}"},
            }
        }
    });
    let limit = limit.unwrap_or(20);
    let section_str = section.map_or_else(|| StackString::from("null"), |s| format_sstr!("'{s}'"));
    let previous_button = if let Some(offset) = offset {
        if *offset < limit {
            None
        } else {
            let new_offset = *offset - limit;
            let section_str = section_str.clone();
            Some(rsx! {
                button {
                    "type": "submit",
                    name: "previous",
                    value: "Previous",
                    "onclick": "loadPlex({new_offset}, {limit}, {section_str})",
                    "Previous",
                }
            })
        }
    } else {
        None
    };
    let offset = offset.unwrap_or(0);
    let new_offset = offset + limit;
    let next_button = rsx! {
        button {
            "type": "submit",
            name: "next",
            value: "Next",
            "onclick": "loadPlex({new_offset}, {limit}, {section_str})",
            "Next",
        }
    };
    let section_select = [
        None,
        Some(PlexSectionType::Music),
        Some(PlexSectionType::Movie),
        Some(PlexSectionType::TvShow),
    ]
    .iter()
    .enumerate()
    .map(|(i, s)| {
        let v = s.map_or_else(|| "", PlexSectionType::to_str);
        if s == section {
            rsx! {
                option {
                    id: "section-{i}",
                    value: "{v}",
                    selected: true,
                    "{v}"
                }
            }
        } else {
            rsx! {
                option {
                    id: "section-{i}",
                    value: "{v}",
                    "{v}"
                }
            }
        }
    });

    cx.render(rsx! {
        br {
            previous_button,
            next_button,
            form {
                action: "javascript:loadPlexSection('plex_section_filter', {offset}, {limit})",
                select {
                    id: "plex_section_filter",
                    "onchange": "loadPlexSection('plex_section_filter', {offset}, {limit})",
                    section_select,
                }
            }
        }
        table {
            "border": "1",
            "align": "center",
            class: "dataframe",
            thead {
                tr {
                    th {"ID"},
                    th {"Time"},
                    th {"Event Type"},
                    th {"Item Type"},
                    th {"Section"},
                    th {"Title"},
                }
            },
            tbody {
                entries
            }
        }
    })
}

pub fn plex_detail_body(
    config: Config,
    event: EventOutput,
    offset: Option<u64>,
    limit: Option<u64>,
) -> String {
    let mut app = VirtualDom::new_with_props(
        plex_detail_element,
        plex_detail_elementProps {
            config,
            event,
            offset,
            limit,
        },
    );
    drop(app.rebuild());
    dioxus_ssr::render(&app)
}

#[inline_props]
fn plex_detail_element(
    cx: Scope,
    config: Config,
    event: EventOutput,
    offset: Option<u64>,
    limit: Option<u64>,
) -> Element {
    let local = DateTimeWrapper::local_tz();
    let id = event.id;
    let last_modified = match config.default_time_zone {
        Some(tz) => {
            let tz = tz.into();
            event.last_modified.to_timezone(tz)
        }
        None => event.last_modified.to_timezone(local),
    };
    let last_modified = last_modified
        .format(format_description!(
            "[year]-[month]-[day]T[hour]:[minute]:[second][offset_hour]:[offset_minute]"
        ))
        .unwrap_or_default();
    let event_str = &event.event;
    let title = &event.title;
    let metadata_type = event.metadata_type.as_ref().map_or("", StackString::as_str);
    let section_title = event.section_title.as_ref().map_or("", StackString::as_str);
    let parent_title = event.parent_title.as_ref().map_or("", StackString::as_str);
    let grandparent_title = event
        .grandparent_title
        .as_ref()
        .map_or("", StackString::as_str);
    let filename = event.filename.as_ref().map_or("", StackString::as_str);

    let limit = limit.unwrap_or(20);
    let offset = offset.unwrap_or(0);

    cx.render(rsx! {
        br {
            button {
                "type": "submit",
                name: "back",
                value: "Back",
                "onclick": "updateMainArticle('/list/plex?limit={limit}&offset={offset}')",
                "Back",
            }
        }
        table {
            "border": "1",
            "align": "center",
            class: "dataframe",
            thead {
                tr {
                    th {"Field"},
                    th {"Value"},
                }
            },
            tbody {
                tr {
                    td {"ID"},
                    td {"{id}"},
                },
                tr {
                    td {"Last Modified"},
                    td {"{last_modified}"},
                },
                tr {
                    td {"Event"},
                    td {"{event_str}"},
                },
                tr {
                    td {"Metadata Type"},
                    td {"{metadata_type}"},
                },
                tr {
                    td {"Section"},
                    td {"{section_title}"},
                },
                tr {
                    td {"Filename"},
                    td {"{filename}"}
                },
                tr {
                    td {"Title"},
                    td {"{title}"},
                },
                tr {
                    td {"Parent Title"},
                    td {"{parent_title}"},
                },
                tr {
                    td {"Grandparent Title"},
                    td {"{grandparent_title}"},
                }
            }
        }
    })
}

/// # Errors
/// Returns error if db query fails
pub fn local_file_body(
    file_lists: FileLists,
    proc_map: HashMap<StackString, Option<ProcStatus>>,
    config: Config,
) -> String {
    let mut app = VirtualDom::new_with_props(
        local_file_element,
        local_file_elementProps {
            file_lists,
            proc_map,
            config,
        },
    );
    drop(app.rebuild());
    dioxus_ssr::render(&app)
}

#[inline_props]
fn local_file_element(
    cx: Scope,
    file_lists: FileLists,
    proc_map: HashMap<StackString, Option<ProcStatus>>,
    config: Config,
) -> Element {
    let file_map = file_lists.get_file_map();
    let entries = file_lists
        .local_file_list
        .iter()
        .enumerate()
        .map(|(idx, f)| {
            let f_key = f
                .replace(".mkv", "")
                .replace(".m4v", "")
                .replace(".avi", "")
                .replace(".mp4", "");
            let button = if file_map.contains_key(f_key.as_str()) {
                rsx! {
                    button {
                        "type": "submit",
                        id: "{f}",
                        "onclick": "cleanup_file('{f}');",
                        "cleanup"
                    }
                }
            } else if let Some(status) = proc_map.get(f_key.as_str()) {
                match status {
                    Some(ProcStatus::Current) => {
                        rsx! {"running"}
                    }
                    Some(ProcStatus::Upcoming) => {
                        rsx! {"upcoming"}
                    }
                    Some(ProcStatus::Finished) => {
                        let mut movie_dirs =
                            movie_directories(config).unwrap_or_else(|_| Vec::new());
                        if f_key.contains("_s") && f_key.contains("_ep") {
                            movie_dirs.insert(0, "".into());
                        }
                        let movie_dirs = movie_dirs.into_iter().enumerate().map(|(i, d)| {
                            rsx! {
                                option {
                                    key: "movie-key-{i}",
                                    value: "{d}",
                                    "{d}",
                                }
                            }
                        });
                        rsx! {
                            select {
                                id: "movie-dir-{f}",
                                movie_dirs,
                            },
                            button {
                                "type": "submit",
                                id: "{f}",
                                "onclick": "remcom_file('{f}')",
                                "move",
                            }
                        }
                    }
                    None => rsx! {"unknown"},
                }
            } else {
                rsx! {
                    button {
                        "type": "submit",
                        id: "{f}",
                        "onclick": "transcode_file('{f}');",
                        "transcode",
                    }
                }
            };

            rsx! {
                tr {
                    key: "flist-key-{idx}",
                    td {"{f}"},
                    td {button},
                }
            }
        });
    cx.render(rsx! {
        br {
            "On-deck Media Files"
        },
        table {
            "border": "1",
            "align": "center",
            class: "dataframe",
            thead {
                tr {
                    th {"File"},
                    th {"Action"},
                }
            },
            tbody {
                entries
            }
        }
    })
}

pub fn procs_html_body(status: TranscodeStatus) -> String {
    let mut app =
        VirtualDom::new_with_props(procs_html_element, procs_html_elementProps { status });
    drop(app.rebuild());
    dioxus_ssr::render(&app)
}

#[inline_props]
fn procs_html_element(cx: Scope, status: TranscodeStatus) -> Element {
    cx.render(procs_html_node(status))
}

pub fn transcode_get_html_body(status: TranscodeStatus) -> String {
    let mut app = VirtualDom::new_with_props(
        transcode_get_html_element,
        transcode_get_html_elementProps { status },
    );
    drop(app.rebuild());
    dioxus_ssr::render(&app)
}

#[inline_props]
fn transcode_get_html_element(cx: Scope, status: TranscodeStatus) -> Element {
    let procs_node = procs_html_node(status);
    cx.render(rsx! {
        br {
            button {
                name: "remcomout",
                id: "remcomoutput",
                "&nbsp;",
            }
        },
        div {
            id: "procs-tables",
            procs_node,
        },
        div {
            id: "local-file-table",
        }
    })
}

fn procs_html_node(status: &TranscodeStatus) -> LazyNodes {
    let proc_headers = ProcInfo::get_header();
    let upcoming_header = TranscodeServiceRequest::get_header();

    let running = if status.procs.is_empty() {
        None
    } else {
        let header = proc_headers.into_iter().enumerate().map(|(i, h)| {
            rsx! {
                th {
                    key: "header-{i}",
                    "{h}",
                }
            }
        });
        let bodies = status
            .procs
            .iter()
            .map(|x| {
                ProcInfo::get_html(x).into_iter().enumerate().map(|(i, b)| {
                    rsx! {
                        td {
                            key: "bodies-b-{i}",
                            "{b}",
                        }
                    }
                })
            })
            .enumerate()
            .map(|(i, b)| {
                rsx! {
                    tr {
                        key: "bodies-{i}",
                        b,
                    }
                }
            });
        Some(rsx! {
            br {
                "Running procs:"
            },
            table {
                "border": "1",
                "align": "center",
                class: "dataframe",
                thead {
                    tr {
                        header,
                    }
                },
                tbody {
                    bodies,
                }
            }
        })
    };
    let upcoming = if status.upcoming_jobs.is_empty() {
        None
    } else {
        let headers = upcoming_header.into_iter().enumerate().map(|(i, h)| {
            rsx! {
                th {
                    key: "upcoming-header-key-{i}",
                    "{h}"
                }
            }
        });
        let bodies = status
            .upcoming_jobs
            .iter()
            .map(|r| {
                TranscodeServiceRequest::get_html(r)
                    .into_iter()
                    .enumerate()
                    .map(|(i, l)| {
                        rsx! {
                            td {
                                key: "upcoming-body-key-td-{i}",
                                "{l}"
                            }
                        }
                    })
            })
            .enumerate()
            .map(|(i, b)| {
                rsx! {
                    tr {
                        key: "upcoming-body-key-{i}",
                        b
                    }
                }
            });
        Some(rsx! {
            br {
                "Upcoming jobs:",
            },
            table {
                "border": "1",
                "align": "center",
                class: "dataframe",
                thead {
                    tr {
                        headers
                    }
                },
                tbody {
                    bodies
                }
            }
        })
    };
    let current = if status.current_jobs.is_empty() {
        None
    } else {
        let jobs = status.current_jobs.iter().enumerate().map(|(i, (_, s))| {
            rsx! {
                br {
                    key: "job-key-{i}",
                    "{s}",
                }
            }
        });
        Some(rsx! {
            br {
                "Current jobs:",
            },
            jobs,
        })
    };
    let finished = if status.finished_jobs.is_empty() {
        None
    } else {
        let jobs = status.finished_jobs.iter().enumerate().map(|(i, f)| {
            let file_name = f
                .file_name()
                .unwrap_or_else(|| OsStr::new(""))
                .to_string_lossy();
            rsx! {
                tr {
                    key: "job-key-{i}",
                    td {
                        "{file_name}",
                    },
                    td {
                        button {
                            "type": "submit",
                            id: "{file_name}",
                            "onclick": "cleanup_file('{file_name}')",
                            "cleanup",
                        }
                    }
                }
            }
        });
        Some(rsx! {
            br {
                "Finished jobs:"
            },
            table {
                "border": "1",
                "align": "center",
                class: "dataframe",
                thead {
                    tr {
                        th {"File"},
                        th {"Action"},
                    }
                },
                tbody {
                    jobs
                }
            }
        })
    };

    rsx! {
        div{
            running,
            upcoming,
            current,
            finished,
        }
    }
}

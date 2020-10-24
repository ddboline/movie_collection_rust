use anyhow::Error;
use stack_string::StackString;
use std::{
    borrow::Borrow,
    collections::HashSet,
    hash::{Hash, Hasher},
};

use crate::{movie_collection::TvShowsResult, tv_show_source::TvShowSource};

#[derive(Debug, Default, Eq)]
pub struct ProcessShowItem {
    pub show: StackString,
    pub title: StackString,
    pub link: StackString,
    pub source: Option<TvShowSource>,
}

impl PartialEq for ProcessShowItem {
    fn eq(&self, other: &Self) -> bool {
        self.link == other.link
    }
}

impl Hash for ProcessShowItem {
    fn hash<H>(&self, state: &mut H)
    where
        H: Hasher,
    {
        self.link.hash(state)
    }
}

impl Borrow<str> for ProcessShowItem {
    fn borrow(&self) -> &str {
        self.link.as_str()
    }
}

impl From<TvShowsResult> for ProcessShowItem {
    fn from(item: TvShowsResult) -> Self {
        Self {
            show: item.show,
            title: item.title,
            link: item.link,
            source: item.source,
        }
    }
}

impl ProcessShowItem {
    pub fn process_shows(
        tvshows: HashSet<Self>,
        watchlist: HashSet<Self>,
    ) -> Result<Vec<StackString>, Error> {
        let watchlist_shows: Vec<_> = watchlist
            .iter()
            .filter(|item| tvshows.get(item.link.as_str()).is_none())
            .collect();

        let mut shows: Vec<_> = tvshows.iter().chain(watchlist_shows.into_iter()).collect();
        shows.sort_by(|x, y| x.show.cmp(&y.show));

        let button_add = r#"<td><button type="submit" id="ID" onclick="watchlist_add('SHOW');">add to watchlist</button></td>"#;
        let button_rm = r#"<td><button type="submit" id="ID" onclick="watchlist_rm('SHOW');">remove from watchlist</button></td>"#;

        let shows: Vec<_> = shows
            .into_iter()
            .map(|item| {
                let has_watchlist = watchlist.contains(item.link.as_str());
                format!(
                    r#"<tr><td>{}</td>
                    <td><a href="https://www.imdb.com/title/{}" target="_blank">imdb</a></td><td>{}</td><td>{}</td><td>{}</td></tr>"#,
                    if tvshows.contains(item.link.as_str()) {
                        format!(r#"<a href="javascript:updateMainArticle('/list/{}')">{}</a>"#, item.show, item.title)
                    } else {
                        format!(
                            r#"<a href="javascript:updateMainArticle('/list/trakt/watched/list/{}')">{}</a>"#,
                            item.link, item.title
                        )
                    },
                    item.link,
                    match item.source {
                        Some(TvShowSource::Netflix) => r#"<a href="https://netflix.com" target="_blank">netflix</a>"#,
                        Some(TvShowSource::Hulu) => r#"<a href="https://hulu.com" target="_blank">hulu</a>"#,
                        Some(TvShowSource::Amazon) => r#"<a href="https://amazon.com" target="_blank">amazon</a>"#,
                        _ => "",
                    },
                    if has_watchlist {
                        format!(r#"<a href="javascript:updateMainArticle('/list/trakt/watched/list/{}')">watchlist</a>"#, item.link)
                    } else {
                        "".to_string()
                    },
                    if has_watchlist {
                        button_rm.replace("SHOW", &item.link)
                    } else {
                        button_add.replace("SHOW", &item.link)
                    },
                ).into()
            })
            .collect();
        Ok(shows)
    }
}

!function() {
    updateMainArticle('/list/cal?source=all');
}();
function updateMainArticle( url) {
    let xmlhttp = new XMLHttpRequest();
    xmlhttp.onload = function f() {
        document.getElementById("main_article").innerHTML = xmlhttp.responseText;
    }
    xmlhttp.open('GET', url, true);
    xmlhttp.send(null);
}
function get_transcode_status() {
    const sleep = ms => new Promise(r => setTimeout(r, ms));

    let xmlhttp = new XMLHttpRequest();
    xmlhttp.onload = function f() {
        document.getElementById("main_article").innerHTML = xmlhttp.responseText;
        update_file_list();
        update_procs();
        sleep(10_000).then(() => {
            update_file_list();
            update_procs();
        });
    }
    xmlhttp.open("GET", '/list/transcode/status', true);
    xmlhttp.send(null);
}
function watched_add(link, season, episode) {
    let url = "/trakt/watched/add/" + link + "/" + season + "/" + episode;
    let xmlhttp = new XMLHttpRequest();
    xmlhttp.open("POST", url, true);
    xmlhttp.onload = function nothing() {
        let url = "/trakt/watched/list/" + link + "/" + season;
        updateMainArticle(url);
    }
    xmlhttp.send(null);
    let out = "requested " + link + "/" + season + "/" + episode
    document.getElementById("remcomoutput").innerHTML = out;
}
function watched_rm(link, season, episode) {
    let url = "/trakt/watched/rm/" + link + "/" + season + "/" + episode
    let xmlhttp = new XMLHttpRequest();
    xmlhttp.open("POST", url, true);
    xmlhttp.onload = function nothing() {
        let url = "/trakt/watched/list/" + link + "/" + season;
        updateMainArticle(url);
    }
    xmlhttp.send(null);
    let out = "requested " + link + "/" + season + "/" + episode
    document.getElementById("remcomoutput").innerHTML = out;
}
function delete_show(index, offset, order_by) {
    let url = "/list/delete/" + index
    let xmlhttp = new XMLHttpRequest();
    xmlhttp.open("DELETE", url, true);
    xmlhttp.onload = function nothing() {
        searchFullQueue(offset, order_by);
    }
    xmlhttp.send(null);
}
function watchlist_add(show) {
    let url = "/trakt/watchlist/add/" + show
    let xmlhttp = new XMLHttpRequest();
    xmlhttp.open("POST", url, true);
    xmlhttp.onload = function nothing() {
        let url = "/list/tvshows";
        updateMainArticle(url);
    }
    xmlhttp.send(null);
    let out = "requested " + link
    document.getElementById("remcomoutput").innerHTML = out;
}
function watchlist_rm(show) {
    let url = "/trakt/watchlist/rm/" + show
    let xmlhttp = new XMLHttpRequest();
    xmlhttp.open("POST", url, true);
    xmlhttp.onload = function nothing() {
        let url = "/list/tvshows";
        updateMainArticle(url);
    }
    xmlhttp.send(null);
    let out = "requested " + link
    document.getElementById("remcomoutput").innerHTML = out;
}
function imdb_update(show, link, season, referal_url) {
    let url = `/list/imdb/${show}?tv=true&update=true&database=true&link=${link}&season=${season}`;
    let xmlhttp = new XMLHttpRequest();
    xmlhttp.open("GET", url, true);
    xmlhttp.onload = function nothing() {
        updateMainArticle(referal_url);
    }
    xmlhttp.send(null);
    let out = "requested " + link
    document.getElementById("remcomoutput").innerHTML = out;
}
function refreshAuth() {
    let url = "/trakt/refresh_auth";
    let xmlhttp = new XMLHttpRequest();
    xmlhttp.onload = function() {
        document.getElementById("main_article").innerHTML = xmlhttp.responseText;
    }
    xmlhttp.open("GET", url, true);
    xmlhttp.send(null);
}
function traktAuth() {
    let url = "/trakt/auth_url";
    let xmlhttp = new XMLHttpRequest();
    xmlhttp.onload = function() {
        let win = window.open(xmlhttp.responseText, '_blank');
        win.focus()
    }
    xmlhttp.open("GET", url, true);
    xmlhttp.send(null);
}
function transcode_queue(index) {
    let url = "/list/transcode/queue/" + index
    let xmlhttp = new XMLHttpRequest();
    xmlhttp.open("POST", url, true);
    xmlhttp.onload = function nothing() {
        document.getElementById("remcomoutput").innerHTML = "&nbsp;";
    }
    xmlhttp.send(null);
    let out = "requested " + index
    document.getElementById("remcomoutput").innerHTML = out;
}
function transcode_queue_directory(index, directory) {
    let url = "/list/transcode/queue/" + directory + "/" + index
    let xmlhttp = new XMLHttpRequest();
    xmlhttp.open("POST", url, true);
    xmlhttp.onload = function nothing() {
        document.getElementById("remcomoutput").innerHTML = "&nbsp;";
    }
    xmlhttp.send(null);
    let out = "requested " + index
    document.getElementById("remcomoutput").innerHTML = out;
}
function update_procs() {
    let url = "/list/transcode/status/procs";
    let xmlhttp = new XMLHttpRequest();
    xmlhttp.open("GET", url, true);
    xmlhttp.onload = function nothing() {
        document.getElementById("procs-tables").innerHTML = xmlhttp.responseText;
    }
    xmlhttp.send(null);
}
function update_file_list() {
    let url = "/list/transcode/status/file_list";
    let xmlhttp = new XMLHttpRequest();
    xmlhttp.open("GET", url, true);
    xmlhttp.onload = function nothing() {
        document.getElementById("local-file-table").innerHTML = xmlhttp.responseText;
    }
    xmlhttp.send(null);
}
function transcode_file(file) {
    let url = "/list/transcode/file/" + file
    let xmlhttp = new XMLHttpRequest();
    xmlhttp.open("POST", url, true);
    xmlhttp.onload = function nothing() {
        update_procs();
        update_file_list();
        document.getElementById("remcomoutput").innerHTML = "&nbsp;";
    }
    xmlhttp.send(null);
    let out = "requested " + file
    document.getElementById("remcomoutput").innerHTML = out;
}
function remcom_file(file) {
    let movie_dir = document.getElementById('movie-dir-' + file).value;
    let url = "/list/transcode/remcom/directory/" + movie_dir + "/" + file;
    if (movie_dir == '') {
        url = "/list/transcode/remcom/file/" + file;
    }
    let xmlhttp = new XMLHttpRequest();
    xmlhttp.open("POST", url, true);
    xmlhttp.onload = function nothing() {
        update_procs();
        update_file_list();
        document.getElementById("remcomoutput").innerHTML = "&nbsp;";
    }
    xmlhttp.send(null);
    let out = "requested " + file
    document.getElementById("remcomoutput").innerHTML = out;
}
function cleanup_file(file) {
    let url = "/list/transcode/cleanup/" + file
    let xmlhttp = new XMLHttpRequest();
    xmlhttp.open("DELETE", url, true);
    xmlhttp.onload = function nothing() {
        update_procs();
        update_file_list();
        document.getElementById("remcomoutput").innerHTML = "&nbsp;";
    }
    xmlhttp.send(null);
    let out = "requested " + file
    document.getElementById("remcomoutput").innerHTML = out;
}
function setSource( link, source_id ) {
    let source = document.getElementById( source_id ).value;
    let url = `/list/imdb_ratings/set_source?link=${link}&source=${source}`;
    let xmlhttp = new XMLHttpRequest();
    xmlhttp.open("GET", url, true);
    xmlhttp.onload = function nothing() {
        updateMainArticle('/trakt/watchlist');
    }
    xmlhttp.send(null);
}
function searchFullQueue(offset, order_by) {
    let search = document.getElementById('full_queue_search').value;
    let url = "/list/full_queue?limit=20" + "&offset=" + offset + "&q=" + search + "&order_by=" + order_by;
    updateMainArticle(url);
}
function loadPlexSection(section_id, offset=0, limit=10) {
    let section = document.getElementById(section_id).value;
    loadPlex(offset, limit, section);
}
function loadPlex(offset=0, limit=10, section=null) {
    let url = '/list/plex?limit=' + limit + '&offset=' + offset;
    if(section) {
        url = url + '&section_type=' + section;
    }
    updateMainArticle(url);
}
function searchTvShows(link) {
    let search = document.getElementById('tv_shows_search').value;
    let url = `${link}&query=${search}`;
    updateMainArticle(url);
}
function sourceTvShows(link) {
    let source = document.getElementById('tv_shows_source_id').value;
    let url = `${link}&source=${source}`;
    updateMainArticle(url);
}
function searchWatchlist(link) {
    let search = document.getElementById('watchlist_search').value;
    let url = `${link}&query=${search}`;
    updateMainArticle(url);
}
function sourceWatchlist(link) {
    let source = document.getElementById('watchlist_source_id').value;
    let url = `${link}&source=${source}`;
    updateMainArticle(url);
}
function extract_subtitles(file) {
    let index = document.getElementById(`subtitle-selector-${file}`).value;
    let url = `/list/transcode/subtitle/${file}/${index}`;
    let xmlhttp = new XMLHttpRequest();
    xmlhttp.open("POST", url, true);
    xmlhttp.onload = function nothing() {
        update_procs();
        update_file_list();
        document.getElementById("remcomoutput").innerHTML = "&nbsp;";
    }
    xmlhttp.send(null);
    let out = `extract ${index} from ${file}`;
    document.getElementById("remcomoutput").innerHTML = out;
}
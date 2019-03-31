#!/usr/bin/python3
# -*- coding: utf-8 -*-
"""
Created on Thu Oct  5 07:43:59 2017

@author: ddboline
"""
from __future__ import (absolute_import, division, print_function, unicode_literals)
import os
import json
from dateutil.parser import parse
import datetime
from threading import Condition
import argparse
from gevent.pywsgi import WSGIServer

from trakt import Trakt

from flask import Flask, request, json

app = Flask(__name__)

list_of_commands = ('list', 'search', 'add', 'cal', 'rm')
help_text = 'commands=%s,[number]' % ','.join(list_of_commands)


def read_credentials():
    credentials = {}
    with open('%s/.trakt/credentials' % os.getenv('HOME')) as f:
        for line in f:
            tmp = line.split('=')
            if len(tmp) > 1:
                key, val = [x.strip() for x in tmp][:2]
                credentials[key] = val
    return credentials


class TraktInstance(object):
    auth_token = '%s/.trakt/auth_token.json' % os.getenv('HOME')

    def __init__(self, username='ddboline'):
        credentials = read_credentials()
        self.username = username
        self.client_id = credentials['client_id']
        self.client_secret = credentials['client_secret']
        self.trakt = Trakt.configuration.defaults.client(
            id=self.client_id, secret=self.client_secret)

        self.is_authenticating = Condition()

        # Bind trakt events
        Trakt.on('oauth.token_refreshed', self.on_token_refreshed)

        self.authorization = self.read_auth()

        if self.authorization is None:
            self.authenticate()

    def authenticate(self):
        if not self.is_authenticating.acquire(blocking=False):
            print('Authentication has already been started')
            return False

        # Request new device code
        code = Trakt['oauth/device'].code()

        print('Enter the code "%s" at %s to authenticate your account' %
              (code.get('user_code'), code.get('verification_url')))

        # Construct device authentication poller
        poller = Trakt['oauth/device'].poll(**code)\
            .on('aborted', self.on_aborted)\
            .on('authenticated', self.on_authenticated)\
            .on('expired', self.on_expired)\
            .on('poll', self.on_poll)

        # Start polling for authentication token
        poller.start(daemon=False)

        # Wait for authentication to complete
        result = self.is_authenticating.wait()
        self.store_auth()
        return result

    def run(self):

        if not self.authorization:
            print('ERROR: Authentication required')
            exit(1)
        else:
            print('authorization:', self.authorization)

        with Trakt.configuration.oauth.from_response(self.authorization, refresh=True):
            # Expired token will be refreshed automatically (as `refresh=True`)
            print(Trakt['sync/watchlist'].shows(pagination=True))

    def get_watchlist_shows(self):
        with Trakt.configuration.oauth.from_response(self.authorization, refresh=True):
            shows = Trakt['sync/watchlist'].shows(pagination=True)
            if shows is None:
                return []
            return [{
                'link': x.get_key('imdb'),
                'title': x.title,
                'year': x.year if x.year is not None else datetime.date.today().year
            } for x in shows.values()]

    def get_watchlist_seasons(self):
        with Trakt.configuration.oauth.from_response(self.authorization, refresh=True):
            return Trakt['sync/watchlist'].seasons(pagination=True)

    def get_watchlist_episodes(self):
        with Trakt.configuration.oauth.from_response(self.authorization, refresh=True):
            return Trakt['sync/watchlist'].episodes(pagination=True)

    def get_watched_shows(self, imdb_id=None):
        with Trakt.configuration.oauth.from_response(self.authorization, refresh=True):
            results = []
            watched = Trakt['sync/watched'].shows(pagination=True)
            if watched is None:
                return []
            for show in watched.values():
                title = show.title
                imdb_url = show.get_key('imdb')
                if imdb_id is not None and imdb_url != imdb_id:
                    continue
                for (season, epi), episode in show.episodes():
                    results.append({
                        'title': title,
                        'imdb_url': imdb_url,
                        'season': season,
                        'episode': epi
                    })
            return results

    def get_watched_movies(self, imdb_id=None):
        with Trakt.configuration.oauth.from_response(self.authorization, refresh=True):
            results = []
            watched = Trakt['sync/watched'].movies(pagination=True)
            if watched is None:
                return []
            for movies in watched.values():
                title = movies.title
                imdb_url = movies.get_key('imdb')
                if imdb_id is not None and imdb_url != imdb_id:
                    continue
                results.append({
                    'title': title,
                    'imdb_url': imdb_url,
                })
            return results

    def on_aborted(self):
        """Device authentication aborted.

        Triggered when device authentication was aborted (either with `DeviceOAuthPoller.stop()`
        or via the "poll" event)
        """

        print('Authentication aborted')

        # Authentication aborted
        self.is_authenticating.acquire()
        self.is_authenticating.notify_all()
        self.is_authenticating.release()

    def on_authenticated(self, authorization):
        """Device authenticated.

        :param authorization: Authentication token details
        :type authorization: dict
        """

        # Acquire condition
        self.is_authenticating.acquire()

        # Store authorization for future calls
        self.authorization = authorization

        print('Authentication successful - authorization: %r' % self.authorization)

        # Authentication complete
        self.is_authenticating.notify_all()
        self.is_authenticating.release()

    def on_expired(self):
        """Device authentication expired."""

        print('Authentication expired')

        # Authentication expired
        self.is_authenticating.acquire()
        self.is_authenticating.notify_all()
        self.is_authenticating.release()

    def on_poll(self, callback):
        """Device authentication poll.

        :param callback: Call with `True` to continue polling, or `False` to abort polling
        :type callback: func
        """

        # Continue polling
        callback(True)

    def on_token_refreshed(self, authorization):
        # OAuth token refreshed, store authorization for future calls
        self.authorization = authorization

        print('Token refreshed - authorization: %r' % self.authorization)

    def store_auth(self):
        with open(self.auth_token, 'w') as f:
            json.dump(self.authorization, f)

    def read_auth(self):
        if not os.path.exists(self.auth_token):
            return None
        with open(self.auth_token, 'r') as f:
            return json.load(f)

    def do_lookup(self, imdb_id):
        with Trakt.configuration.oauth.from_response(self.authorization, refresh=True):
            return Trakt['search'].lookup(id=imdb_id, service='imdb')

    def do_query(self, show, media='show'):
        with Trakt.configuration.oauth.from_response(self.authorization, refresh=True):
            shows = Trakt['search'].query(show.replace('_', ' '), media=media, pagination=True)
            shows = {s.get_key('imdb'): s.to_dict() for s in shows}
            return shows

    def add_show_to_watchlist(self, show=None, imdb_id=None):
        if imdb_id:
            show_obj = self.do_lookup(imdb_id)
        elif show:
            show_obj = self.do_query(show)
        if isinstance(show_obj, list):
            if len(show_obj) < 1:
                return {}
            else:
                show_obj = show_obj[0]
        if show_obj is None:
            return {}
        with Trakt.configuration.oauth.from_response(self.authorization, refresh=True):
            items = {'shows': [show_obj.to_dict()]}
            return Trakt['sync/watchlist'].add(items=items)

    def add_episode_to_watched(self, show=None, imdb_id=None, season=None, episode=None):
        if imdb_id:
            show_obj = self.do_lookup(imdb_id)
        elif show:
            show_obj = self.do_query(show)
        if show_obj is None:
            return {}
        if isinstance(show_obj, list):
            if len(show_obj) < 1:
                return {}
            else:
                show_obj = show_obj[0]
        if season and episode:
            print(show_obj)
            episode_ = Trakt['shows'].episode(
                show_obj.get_key('imdb'), season=season, episode=episode)
            if not episode_:
                return {}
            with Trakt.configuration.oauth.from_response(self.authorization, refresh=True):
                items = {'episodes': [episode_.to_dict()]}
                return Trakt['sync/history'].add(items=items)
        elif season:
            with Trakt.configuration.oauth.from_response(self.authorization, refresh=True):
                episodes = Trakt['shows'].season(show_obj.get_key('imdb'), season=season)
                if not episodes:
                    return {}
                episodes = [e.to_dict() for e in episodes]
                items = {'episodes': episodes}
                return Trakt['sync/history'].add(items=items)

    def remove_show_to_watchlist(self, show=None, imdb_id=None):
        if imdb_id:
            show_obj = self.do_lookup(imdb_id)
        elif show:
            show_obj = self.do_query(show)
        if isinstance(show_obj, list):
            if len(show_obj) < 1:
                return {}
            else:
                show_obj = show_obj[0]
        with Trakt.configuration.oauth.from_response(self.authorization, refresh=True):
            items = {'shows': [show_obj.to_dict()]}
            print(show_obj)
            return Trakt['sync/watchlist'].remove(items=items)

    def remove_movie_to_watched(self, show=None, imdb_id=None):
        if imdb_id:
            show_obj = self.do_lookup(imdb_id)
        elif show:
            show_obj = self.do_query(show)
        if isinstance(show_obj, list):
            if len(show_obj) < 1:
                return
            else:
                show_obj = show_obj[0]
        with Trakt.configuration.oauth.from_response(self.authorization, refresh=True):
            items = {'movies': [show_obj.to_dict()]}
            return Trakt['sync/history'].remove(items=items)

    def remove_episode_to_watched(self, show=None, imdb_id=None, season=None, episode=None):
        if imdb_id:
            show_obj = self.do_lookup(imdb_id)
        elif show:
            show_obj = self.do_query(show)
        if isinstance(show_obj, list):
            if len(show_obj) < 1:
                return
            else:
                show_obj = show_obj[0]
        if season and episode:
            episode_ = Trakt['shows'].episode(
                show_obj.get_key('imdb'), season=season, episode=episode)
            with Trakt.configuration.oauth.from_response(self.authorization, refresh=True):
                items = {'episodes': [episode_.to_dict()]}
                print(episode_)
                return Trakt['sync/history'].remove(items=items)
        elif season:
            with Trakt.configuration.oauth.from_response(self.authorization, refresh=True):
                episodes = []
                for episode_ in Trakt['shows'].season(show_obj.get_key('imdb'), season=season):
                    episodes.append(episode_.to_dict())
                items = {'episodes': episodes}
                print(episodes)
                return Trakt['sync/history'].remove(items=items)

    def add_movie_to_watched(self, title=None, imdb_id=None):
        if imdb_id:
            show_obj = self.do_lookup(imdb_id)
        elif title:
            show_obj = self.do_query(title)
        if isinstance(show_obj, list):
            if len(show_obj) < 1:
                return {}
            else:
                show_obj = show_obj[0]
        if isinstance(show_obj, dict):
            if not show_obj:
                return {}
            show_obj.values()[0]
        with Trakt.configuration.oauth.from_response(self.authorization, refresh=True):
            items = {'movies': [show_obj.to_dict()]}
            return Trakt['sync/history'].add(items=items)

    def get_calendar(self):
        with Trakt.configuration.oauth.from_response(self.authorization, refresh=True):
            return Trakt['calendars/my/*'].get(media='shows', pagination=True)

    def get_trakt_cal_episode(self):
        maxdate = datetime.date.today() + datetime.timedelta(days=90)

        output = []
        cal = self.get_calendar()
        if cal is None:
            return []
        for ep_ in cal:
            show = ep_.show.title
            season, episode = ep_.pk
            airdate = ep_.first_aired.date()

            if airdate > maxdate:
                continue

            airdate = airdate.isoformat()

            imdb_url = ep_.show.get_key('imdb')
            ep_url = ep_.get_key('imdb')

            eprating, rating = -1, -1
            title = show
            eptitle = ep_.title
            output.append({
                'show': ep_.show.title,
                'link': imdb_url,
                'season': season,
                'episode': episode,
                'ep_link': ep_url,
                'airdate': airdate
            })
        return output


### return {<imdb_id>, {"title": <title>, "year": <year>}}
@app.route('/trakt/watchlist', methods=['GET'])
def get_watchlist():
    ti = TraktInstance()
    return json.jsonify(ti.get_watchlist_shows()), 200


@app.route('/trakt/watched_show/<imdb_id>', methods=['GET'])
def get_watched_show(imdb_id):
    ti = TraktInstance()
    return json.jsonify(ti.get_watched_shows(imdb_id=imdb_id)), 200


@app.route('/trakt/watched_shows', methods=['GET'])
def get_watched_shows():
    ti = TraktInstance()
    return json.jsonify(ti.get_watched_shows()), 200


@app.route('/trakt/watched_movie/<imdb_id>', methods=['GET'])
def get_watched_movie(imdb_id):
    ti = TraktInstance()
    return json.jsonify(ti.get_watched_movies(imdb_id=imdb_id)), 200


@app.route('/trakt/watched_movies', methods=['GET'])
def get_watched_movies():
    ti = TraktInstance()
    return json.jsonify(ti.get_watched_movies()), 200


@app.route('/trakt/query/<query_str>', methods=['GET'])
def do_trakt_query(query_str):
    ti = TraktInstance()
    return json.jsonify(ti.do_query(query_str)), 200


@app.route('/trakt/lookup/<imdb_id>', methods=['GET'])
def do_trakt_lookup(imdb_id):
    ti = TraktInstance()
    return json.jsonify(ti.do_lookup(imdb_id=imdb_id).to_dict()), 200


@app.route('/trakt/add_episode_to_watched/<imdb_id>/<season>/<episode>', methods=['GET'])
def add_episode_to_watched(imdb_id, season, episode):
    ti = TraktInstance()
    result = ti.add_episode_to_watched(imdb_id=imdb_id, season=int(season), episode=int(episode))
    if result:
        return json.jsonify({'status': 'success'}), 200
    else:
        return json.jsonify({'status': 'failure'}), 200


@app.route('/trakt/add_to_watched/<imdb_id>', methods=['GET'])
def add_to_watched(imdb_id):
    ti = TraktInstance()
    result = ti.add_movie_to_watched(imdb_id=imdb_id)
    if result:
        return json.jsonify({'status': 'success'}), 200
    else:
        return json.jsonify({'status': 'failure'}), 200


@app.route('/trakt/cal', methods=['GET'])
def get_trakt_cal():
    ti = TraktInstance()
    return json.jsonify(ti.get_trakt_cal_episode()), 200


@app.route('/trakt/delete_show/<imdb_id>', methods=['GET'])
def delete_show_from_watchlist(imdb_id):
    ti_ = TraktInstance()
    result = ti_.remove_show_to_watchlist(imdb_id=imdb_id)
    if result:
        return json.jsonify({'status': 'success'}), 200
    else:
        return json.jsonify({'status': 'failure'}), 200


@app.route('/trakt/delete_watched/<imdb_id>/<season>/<episode>', methods=['GET'])
def delete_episode_from_watched(imdb_id, season, episode):
    ti_ = TraktInstance()
    result = ti_.remove_episode_to_watched(
        imdb_id=imdb_id, season=int(season), episode=int(episode))
    if result:
        return json.jsonify({'status': 'success'}), 200
    else:
        return json.jsonify({'status': 'failure'}), 200


@app.route('/trakt/delete_watched_movie/<imdb_id>', methods=['GET'])
def delete_movie_from_watched(imdb_id):
    ti_ = TraktInstance()
    result = ti_.remove_movie_to_watched(imdb_id=imdb_id)
    if result:
        return json.jsonify({'status': 'success'}), 200
    else:
        return json.jsonify({'status': 'failure'}), 200


@app.route('/trakt/add_to_watchlist/<imdb_id>', methods=['GET'])
def add_to_watchlist(imdb_id):
    ti = TraktInstance()
    result = ti.add_show_to_watchlist(imdb_id=imdb_id)
    if result:
        return json.jsonify({'status': 'success'}), 200
    else:
        return json.jsonify({'status': 'failure'}), 200


if __name__ == '__main__':
    http_server = WSGIServer(('', 32458), app)
    http_server.serve_forever()

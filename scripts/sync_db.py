#!/usr/bin/python3
import requests
import os
from urllib.parse import urlencode


def read_config_env():
    with open('config.env', 'r') as f:
        for l in f:
            (key, val) = l.strip().split('=')[:2]
            os.environ[key] = val


read_config_env()

client_id = os.environ['FITBIT_CLIENTID']
client_secret = os.environ['FITBIT_CLIENTSECRET']
garmin_username = os.environ['GARMIN_USERNAME']
garmin_password = os.environ['GARMIN_PASSWORD']

entry_map = {
    'imdb_episodes': 'episodes',
    'imdb_ratings': 'shows',
    'movie_collection': 'collection',
    'movie_queue': 'queue'
}

dry_run = True


def sync_db():
    endpoint0 = 'https://www.ddboline.net'
    endpoint1 = 'https://cloud.ddboline.net'

    cookies0 = requests.post(
        f'{endpoint0}/api/auth', json={
            'email': garmin_username,
            'password': garmin_password
        }).cookies
    cookies1 = requests.post(
        f'{endpoint1}/api/auth', json={
            'email': garmin_username,
            'password': garmin_password
        }).cookies

    last_modified0 = requests.get(f'{endpoint0}/list/last_modified', cookies=cookies0).json()
    last_modified1 = requests.get(f'{endpoint1}/list/last_modified', cookies=cookies1).json()

    last_modified0 = {x['table']: x['last_modified'] for x in last_modified0}
    last_modified1 = {x['table']: x['last_modified'] for x in last_modified1}

    tables = set(last_modified0) & set(last_modified1)

    for table in tables:
        print(table)
        mod0 = last_modified0[table]
        mod1 = last_modified1[table]

        print(mod0, mod1)
        if mod0 > mod1:
            endpoint = f'{endpoint0}/list/{table}?%s' % urlencode({
                'start_timestamp': mod1,
            })
            print(endpoint)
            resp = requests.get(endpoint, cookies=cookies0).json()
            if not dry_run:
                endpoint = f'{endpoint1}/list/{table}'
                js = {entry_map[table]: resp}
                resp = requests.post(endpoint, json=js, cookies=cookies1)
                print(resp.status_code, resp.text)
        if mod1 > mod0:
            endpoint = f'{endpoint1}/list/{table}?%s' % urlencode({
                'start_timestamp': mod0,
            })
            print(endpoint)
            resp = requests.get(endpoint, cookies=cookies1).json()
            if not dry_run:
                endpoint = f'{endpoint0}/list/{table}'
                js = {entry_map[table]: resp}
                resp = requests.post(endpoint, json=js, cookies=cookies0)
                print(resp.status_code, resp.text)

    return


if __name__ == '__main__':
    sync_db()

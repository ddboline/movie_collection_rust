pub use authorized_users::{
    get_random_key, get_secrets, token::Token, AuthorizedUser, AUTHORIZED_USERS, JWT_SECRET,
    KEY_LENGTH, SECRET_KEY, TRIGGER_DB_UPDATE,
};
use log::debug;
use reqwest::{header::HeaderValue, Client};
use rweb::{filters::cookie::cookie, Filter, Rejection, Schema};
use serde::{Deserialize, Serialize};
use stack_string::{format_sstr, StackString};
use std::{
    convert::{TryFrom, TryInto},
    env::var,
    fmt::Write,
    str::FromStr,
};
use tokio::task::{spawn, JoinHandle};
use uuid::Uuid;

use movie_collection_lib::{config::Config, pgpool::PgPool, utils::get_authorized_users};

use crate::errors::ServiceError as Error;

#[derive(Serialize, Deserialize)]
struct SessionData {
    movie_last_url: StackString,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Hash, Clone, Schema)]
pub struct LoggedUser {
    #[schema(description = "Email Address")]
    pub email: StackString,
    #[schema(description = "Session UUID")]
    pub session: Uuid,
    #[schema(description = "Secret Key")]
    pub secret_key: StackString,
}

impl LoggedUser {
    pub fn verify_session_id(&self, session_id: Uuid) -> Result<(), Error> {
        if self.session == session_id {
            Ok(())
        } else {
            Err(Error::Unauthorized)
        }
    }

    pub fn filter() -> impl Filter<Extract = (Self,), Error = Rejection> + Copy {
        cookie("session-id")
            .and(cookie("jwt"))
            .and_then(|id: Uuid, user: Self| async move {
                user.verify_session_id(id)
                    .map(|_| user)
                    .map_err(rweb::reject::custom)
            })
    }

    pub async fn get_url(
        &self,
        client: &Client,
        config: &Config,
    ) -> Result<Option<StackString>, anyhow::Error> {
        let url = format_sstr!("https://{}/api/session/movie-queue", config.domain);
        let session_str = StackString::from_display(self.session);
        let value = HeaderValue::from_str(&session_str)?;
        let key = HeaderValue::from_str(&self.secret_key)?;
        let session: Option<SessionData> = client
            .get(url.as_str())
            .header("session", value)
            .header("secret-key", key)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(session.map(|x| x.movie_last_url))
    }

    pub async fn set_url(
        &self,
        client: &Client,
        config: &Config,
        set_url: &str,
    ) -> Result<(), anyhow::Error> {
        let url = format_sstr!("https://{}/api/session/movie-queue", config.domain);
        let session_str = StackString::from_display(self.session);
        let value = HeaderValue::from_str(&session_str)?;
        let key = HeaderValue::from_str(&self.secret_key)?;
        let session = SessionData {
            movie_last_url: set_url.into(),
        };
        client
            .post(url.as_str())
            .header("session", value)
            .header("secret-key", key)
            .json(&session)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    pub async fn store_url_task(
        self,
        client: &Client,
        config: &Config,
        set_url: &str,
    ) -> JoinHandle<Option<()>> {
        spawn({
            let client = client.clone();
            let config = config.clone();
            let set_url = set_url.to_owned();
            async move { self.set_url(&client, &config, &set_url).await.ok() }
        })
    }
}

impl From<AuthorizedUser> for LoggedUser {
    fn from(user: AuthorizedUser) -> Self {
        Self {
            email: user.email,
            session: user.session,
            secret_key: user.secret_key,
        }
    }
}

impl TryFrom<Token> for LoggedUser {
    type Error = Error;
    fn try_from(token: Token) -> Result<Self, Self::Error> {
        let user = token.try_into()?;
        if AUTHORIZED_USERS.is_authorized(&user) {
            Ok(user.into())
        } else {
            debug!("NOT AUTHORIZED {:?}", user);
            Err(Error::Unauthorized)
        }
    }
}

impl FromStr for LoggedUser {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut buf = StackString::new();
        buf.push_str(s);
        let token: Token = buf.into();
        token.try_into()
    }
}

pub async fn fill_from_db(pool: &PgPool) -> Result<(), Error> {
    debug!("{:?}", *TRIGGER_DB_UPDATE);
    let users = if TRIGGER_DB_UPDATE.check() {
        get_authorized_users(pool).await?
    } else {
        AUTHORIZED_USERS.get_users()
    };
    if let Ok("true") = var("TESTENV").as_ref().map(String::as_str) {
        AUTHORIZED_USERS.merge_users(["user@test"])?;
    }
    AUTHORIZED_USERS.merge_users(users)?;

    debug!("{:?}", *AUTHORIZED_USERS);
    Ok(())
}

pub use authorized_users::{
    get_random_key, get_secrets, token::Token, AuthorizedUser as ExternalUser, AUTHORIZED_USERS,
    JWT_SECRET, KEY_LENGTH, LOGIN_HTML, SECRET_KEY, TRIGGER_DB_UPDATE,
};
use futures::TryStreamExt;
use log::debug;
use maplit::hashmap;
use reqwest::Client;
use rweb::{filters::cookie::cookie, Filter, Rejection, Schema};
use rweb_helper::{DateTimeType, UuidWrapper};
use serde::{Deserialize, Serialize};
use stack_string::{format_sstr, StackString};
use std::{
    collections::HashMap,
    convert::{TryFrom, TryInto},
    env::var,
    str::FromStr,
};
use time::OffsetDateTime;
use tokio::task::{spawn, JoinHandle};
use url::Url;
use uuid::Uuid;

use movie_collection_lib::{config::Config, pgpool::PgPool, utils::AuthorizedUsers};

use crate::errors::ServiceError as Error;

#[derive(Serialize, Deserialize)]
struct SessionData {
    movie_last_url: StackString,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Hash, Clone, Schema)]
#[schema(component = "LoggedUser")]
pub struct LoggedUser {
    #[schema(description = "Email Address")]
    pub email: StackString,
    #[schema(description = "Session UUID")]
    pub session: UuidWrapper,
    #[schema(description = "Secret Key")]
    pub secret_key: StackString,
    #[schema(description = "User Created At")]
    pub created_at: DateTimeType,
}

impl LoggedUser {
    /// # Errors
    /// Return error if `session_id` does not match `self.session`
    pub fn verify_session_id(&self, session_id: Uuid) -> Result<(), Error> {
        if self.session == session_id {
            Ok(())
        } else {
            Err(Error::Unauthorized)
        }
    }

    #[must_use]
    pub fn filter() -> impl Filter<Extract = (Self,), Error = Rejection> + Copy {
        cookie("session-id")
            .and(cookie("jwt"))
            .and_then(|id: Uuid, user: Self| async move {
                user.verify_session_id(id)
                    .map(|()| user)
                    .map_err(rweb::reject::custom)
            })
    }

    /// # Errors
    /// Returns error if api call fails
    pub async fn get_url(
        &self,
        client: &Client,
        config: &Config,
    ) -> Result<Option<StackString>, anyhow::Error> {
        let base_url: Url = format_sstr!("https://{}", config.domain).parse()?;
        let session: Option<SessionData> = ExternalUser::get_session_data(
            &base_url,
            self.session.into(),
            &self.secret_key,
            client,
            "movie-queue",
        )
        .await?;
        Ok(session.map(|x| x.movie_last_url))
    }

    /// # Errors
    /// Returns error if api call fails
    pub async fn set_url(
        &self,
        client: &Client,
        config: &Config,
        set_url: &str,
    ) -> Result<(), anyhow::Error> {
        let base_url: Url = format_sstr!("https://{}", config.domain).parse()?;
        let session = SessionData {
            movie_last_url: set_url.into(),
        };
        ExternalUser::set_session_data(
            &base_url,
            self.session.into(),
            &self.secret_key,
            client,
            "movie-queue",
            &session,
        )
        .await?;
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

impl From<ExternalUser> for LoggedUser {
    fn from(user: ExternalUser) -> Self {
        Self {
            email: user.email,
            session: user.session.into(),
            secret_key: user.secret_key,
            created_at: user.created_at.into(),
        }
    }
}

impl TryFrom<Token> for LoggedUser {
    type Error = Error;
    fn try_from(token: Token) -> Result<Self, Self::Error> {
        if let Ok(user) = token.try_into() {
            if AUTHORIZED_USERS.is_authorized(&user) {
                return Ok(user.into());
            }
            debug!("NOT AUTHORIZED {:?}", user);
        }
        Err(Error::Unauthorized)
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

/// # Errors
/// Returns error if `get_authorized_users` fails
pub async fn fill_from_db(pool: &PgPool) -> Result<(), Error> {
    if let Ok("true") = var("TESTENV").as_ref().map(String::as_str) {
        AUTHORIZED_USERS.update_users(hashmap! {
            "user@test".into() => ExternalUser {
                email: "user@test".into(),
                session: Uuid::new_v4(),
                secret_key: StackString::default(),
                created_at: OffsetDateTime::now_utc()
            }
        });
        return Ok(());
    }
    let (created_at, deleted_at) = AuthorizedUsers::get_most_recent(pool).await?;
    let most_recent_user_db = created_at.max(deleted_at);
    let existing_users = AUTHORIZED_USERS.get_users();
    let most_recent_user = existing_users.values().map(|i| i.created_at).max();
    debug!("most_recent_user_db {most_recent_user_db:?} most_recent_user {most_recent_user:?}");
    if most_recent_user_db.is_some()
        && most_recent_user.is_some()
        && most_recent_user_db <= most_recent_user
    {
        return Ok(());
    }

    let result: Result<HashMap<StackString, _>, _> = AuthorizedUsers::get_authorized_users(pool)
        .await?
        .map_ok(|u| {
            (
                u.email.clone(),
                ExternalUser {
                    email: u.email,
                    session: Uuid::new_v4(),
                    secret_key: StackString::default(),
                    created_at: u.created_at,
                },
            )
        })
        .try_collect()
        .await;
    let users = result?;
    AUTHORIZED_USERS.update_users(users);
    debug!("AUTHORIZED_USERS {:?}", *AUTHORIZED_USERS);
    Ok(())
}

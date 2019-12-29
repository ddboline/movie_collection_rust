#![allow(clippy::needless_pass_by_value)]

use actix_web::web::{block, Data};
use actix_web::HttpResponse;
use failure::{err_msg, Error};
use futures::future::Future;
use std::collections::{HashMap, HashSet};

use super::logged_user::LoggedUser;
use super::movie_queue_app::AppState;
use super::HandleRequest;

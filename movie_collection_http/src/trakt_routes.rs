#![allow(clippy::needless_pass_by_value)]

use actix_web::web::{block, Data, Path};
use actix_web::HttpResponse;
use failure::{err_msg, Error};
use futures::Future;
use std::collections::HashMap;

use super::logged_user::LoggedUser;
use super::movie_queue_app::AppState;
use super::movie_queue_requests::{
    
};
use super::HandleRequest;
use super::{form_http_response, generic_route};

use actix_threadpool::BlockingError;
use actix_web::{error::ResponseError, HttpResponse};
use anyhow::Error as AnyhowError;
use handlebars::RenderError;
use rust_auth_server::static_files::login_html;
use stack_string::StackString;
use std::fmt::Debug;
use subprocess::PopenError;
use thiserror::Error;

use crate::logged_user::TRIGGER_DB_UPDATE;

#[derive(Error, Debug)]
#[allow(clippy::used_underscore_binding)]
pub enum ServiceError {
    #[error("Internal Server Error")]
    InternalServerError,
    #[error("BadRequest: {0}")]
    BadRequest(StackString),
    #[error("Unauthorized")]
    Unauthorized,
    #[error("Anyhow error {0}")]
    AnyhowError(#[from] AnyhowError),
    #[error("blocking error {0}")]
    BlockingError(StackString),
    #[error("Popen error {0}")]
    PopenError(#[from] PopenError),
    #[error("Template Parse Error {0}")]
    RenderError(#[from] RenderError),
}

// impl ResponseError trait allows to convert our errors into http responses
// with appropriate data
impl ResponseError for ServiceError {
    fn error_response(&self) -> HttpResponse {
        match *self {
            Self::BadRequest(ref message) => HttpResponse::BadRequest().json(message),
            Self::Unauthorized => {
                TRIGGER_DB_UPDATE.set();
                login_html()
            }
            _ => {
                HttpResponse::InternalServerError().json("Internal Server Error, Please try later")
            }
        }
    }
}

impl<T: Debug> From<BlockingError<T>> for ServiceError {
    fn from(item: BlockingError<T>) -> Self {
        Self::BlockingError(format!("{:?}", item).into())
    }
}

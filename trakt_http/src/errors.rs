use actix_web::{error::ResponseError, HttpResponse};
use anyhow::Error as AnyhowError;
use handlebars::RenderError;
use reqwest::Error as ReqwestError;
use stack_string::StackString;
use std::{fmt::Debug, io::Error as IoError};
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
    #[error("Template Parse Error {0}")]
    RenderError(#[from] RenderError),
    #[error("IoError {0}")]
    IoError(#[from] IoError),
}

// impl ResponseError trait allows to convert our errors into http responses
// with appropriate data
impl ResponseError for ServiceError {
    fn error_response(&self) -> HttpResponse {
        match self {
            Self::BadRequest(message) => HttpResponse::BadRequest().json(message),
            Self::Unauthorized => {
                TRIGGER_DB_UPDATE.set();
                login_html()
            }
            Self::AnyhowError(e) => {
                if let Some(err) = e.downcast_ref::<ReqwestError>() {
                    if let Some(status) = err.status() {
                        let url = err.url().map_or("", |u| u.as_str());
                        return HttpResponse::InternalServerError().json(format!(
                            "Status code {} for {}",
                            status.as_str(),
                            url
                        ));
                    }
                }
                HttpResponse::InternalServerError().json("Internal Server Error, Please try later")
            }
            _ => {
                HttpResponse::InternalServerError().json("Internal Server Error, Please try later")
            }
        }
    }
}

fn login_html() -> HttpResponse {
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(
            "
            <script>
                !function() {
                    let final_url = location.href;
                    location.replace('/auth/login.html?final_url=' + final_url);
                }()
            </script>
        ",
        )
}

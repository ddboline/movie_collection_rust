use anyhow::Error as AnyhowError;
use axum::{
    extract::Json,
    http::{
        header::{InvalidHeaderName, InvalidHeaderValue, ToStrError, CONTENT_TYPE},
        StatusCode,
    },
    response::{IntoResponse, Response},
};
use log::error;
use postgres_query::Error as PqError;
use serde::Serialize;
use serde_json::Error as SerdeJsonError;
use serde_yml::Error as YamlError;
use stack_string::{format_sstr, StackString};
use std::{
    fmt::{Debug, Error as FmtError},
    io::Error as IoError,
    net::AddrParseError,
};
use thiserror::Error;
use utoipa::{
    openapi::{content::ContentBuilder, ResponseBuilder, ResponsesBuilder},
    IntoResponses, PartialSchema, ToSchema,
};

use authorized_users::errors::AuthUsersError;

use crate::logged_user::LOGIN_HTML;

#[derive(Error, Debug)]
#[allow(clippy::used_underscore_binding)]
pub enum ServiceError {
    #[error("ToStrError {0}")]
    ToStrError(#[from] ToStrError),
    #[error("InvalidHeaderName {0}")]
    InvalidHeaderName(#[from] InvalidHeaderName),
    #[error("InvalidHeaderValue {0}")]
    InvalidHeaderValue(#[from] InvalidHeaderValue),
    #[error("AuthUsersError {0}")]
    AuthUsersError(#[from] AuthUsersError),
    #[error("Internal Server Error")]
    InternalServerError,
    #[error("BadRequest: {0}")]
    BadRequest(StackString),
    #[error("Unauthorized")]
    Unauthorized,
    #[error("Anyhow error {0}")]
    AnyhowError(#[from] AnyhowError),
    #[error("IoError {0}")]
    IoError(#[from] IoError),
    #[error("FmtError {0}")]
    FmtError(#[from] FmtError),
    #[error("PqError {0}")]
    PqError(Box<PqError>),
    #[error("SerdeJsonError {0}")]
    SerdeJsonError(#[from] SerdeJsonError),
    #[error("YamlError {0}")]
    YamlError(#[from] YamlError),
    #[error("AddrParseError {0}")]
    AddrParseError(#[from] AddrParseError),
}

impl From<PqError> for ServiceError {
    fn from(value: PqError) -> Self {
        Self::PqError(value.into())
    }
}

#[derive(Serialize, ToSchema)]
struct ErrorMessage {
    message: StackString,
}

impl axum::response::IntoResponse for ErrorMessage {
    fn into_response(self) -> Response {
        Json(self).into_response()
    }
}

impl IntoResponse for ServiceError {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthorized | Self::AuthUsersError(AuthUsersError::InvalidHeaderValue(_)) => (
                StatusCode::OK,
                [(CONTENT_TYPE, mime::TEXT_HTML.essence_str())],
                LOGIN_HTML,
            )
                .into_response(),
            Self::BadRequest(s) => (
                StatusCode::BAD_REQUEST,
                [(CONTENT_TYPE, mime::APPLICATION_JSON.essence_str())],
                ErrorMessage { message: s },
            )
                .into_response(),
            e => (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(CONTENT_TYPE, mime::APPLICATION_JSON.essence_str())],
                ErrorMessage {
                    message: format_sstr!("Internal Server Error: {e}"),
                },
            )
                .into_response(),
        }
    }
}

impl IntoResponses for ServiceError {
    fn responses() -> std::collections::BTreeMap<
        String,
        utoipa::openapi::RefOr<utoipa::openapi::response::Response>,
    > {
        let error_message_content = ContentBuilder::new()
            .schema(Some(ErrorMessage::schema()))
            .build();
        ResponsesBuilder::new()
            .response(
                StatusCode::UNAUTHORIZED.as_str(),
                ResponseBuilder::new()
                    .description("Not Authorized")
                    .content(
                        mime::TEXT_HTML.essence_str(),
                        ContentBuilder::new().schema(Some(String::schema())).build(),
                    ),
            )
            .response(
                StatusCode::BAD_REQUEST.as_str(),
                ResponseBuilder::new().description("Bad Request").content(
                    mime::APPLICATION_JSON.essence_str(),
                    error_message_content.clone(),
                ),
            )
            .response(
                StatusCode::INTERNAL_SERVER_ERROR.as_str(),
                ResponseBuilder::new()
                    .description("Internal Server Error")
                    .content(
                        mime::APPLICATION_JSON.essence_str(),
                        error_message_content.clone(),
                    ),
            )
            .build()
            .into()
    }
}

#[cfg(test)]
mod test {
    use anyhow::Error as AnyhowError;
    use axum::http::header::{InvalidHeaderName, InvalidHeaderValue, ToStrError};
    use postgres_query::Error as PqError;
    use serde_json::Error as SerdeJsonError;
    use serde_yml::Error as YamlError;
    use std::{fmt::Error as FmtError, io::Error as IoError, net::AddrParseError};
    use tokio::{task::JoinError, time::error::Elapsed};
    use url::ParseError as UrlParseError;

    use authorized_users::errors::AuthUsersError;

    use crate::errors::ServiceError as Error;

    #[test]
    fn test_error_size() {
        println!(
            "InvalidHeaderName {}",
            std::mem::size_of::<InvalidHeaderName>()
        );
        println!(
            "InvalidHeaderValue {}",
            std::mem::size_of::<InvalidHeaderValue>()
        );
        println!("YamlError {}", std::mem::size_of::<YamlError>());
        println!("AddrParseError {}", std::mem::size_of::<AddrParseError>());
        println!("ToStrError {}", std::mem::size_of::<ToStrError>());
        println!("JoinError {}", std::mem::size_of::<JoinError>());
        println!("UrlParseError {}", std::mem::size_of::<UrlParseError>());
        println!("SerdeJsonError {}", std::mem::size_of::<SerdeJsonError>());
        println!("Elapsed {}", std::mem::size_of::<Elapsed>());
        println!("FmtError {}", std::mem::size_of::<FmtError>());
        println!("AuthUsersError {}", std::mem::size_of::<AuthUsersError>());
        println!("AnyhowError {}", std::mem::size_of::<AnyhowError>());
        println!("IoError {}", std::mem::size_of::<IoError>());
        println!("FmtError {}", std::mem::size_of::<FmtError>());
        println!("PqError {}", std::mem::size_of::<PqError>());

        assert_eq!(std::mem::size_of::<Error>(), 24);
    }
}

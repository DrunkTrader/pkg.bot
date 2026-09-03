pub mod packages;

use std::sync::Arc;

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;

use crate::manager::Manager;

/// Application context passed to all handlers.
pub struct Ctx {
    pub mgr: Arc<Manager>,
    pub consts: Consts,
}

/// Application constants.
#[derive(Clone, Serialize)]
pub struct Consts {
    pub root_url: String,

    // API pagination settings.
    pub api_default_per_page: i32,
    pub api_max_per_page: i32,
}

/// API response wrapper.
#[derive(Serialize)]
pub struct ApiResp<T> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub data: Option<T>,
}

impl<T: Serialize> IntoResponse for ApiResp<T> {
    fn into_response(self) -> Response {
        (StatusCode::OK, Json(self)).into_response()
    }
}

pub fn json<T: Serialize>(data: T) -> ApiResp<T> {
    ApiResp {
        data: Some(data),
        message: None,
    }
}

/// API error type.
#[derive(Debug)]
pub struct ApiErr {
    pub message: String,
    pub status: StatusCode,
}

impl ApiErr {
    pub fn new(message: impl Into<String>, status: StatusCode) -> Self {
        Self {
            message: message.into(),
            status,
        }
    }
}

impl<E: std::fmt::Display> From<E> for ApiErr {
    fn from(err: E) -> Self {
        Self::new(err.to_string(), StatusCode::INTERNAL_SERVER_ERROR)
    }
}

impl IntoResponse for ApiErr {
    fn into_response(self) -> Response {
        let json = Json(ApiResp::<()> {
            data: None,
            message: Some(self.message),
        });
        (self.status, json).into_response()
    }
}

pub type Result<T> = std::result::Result<T, ApiErr>;

/// Pagination helper.
pub fn paginate(
    page: i32,
    per_page: i32,
    max_per_page: i32,
    default_per_page: i32,
) -> (i32, i32, i32) {
    let page = if page < 1 { 1 } else { page };
    let per_page = if per_page < 1 {
        default_per_page
    } else if per_page > max_per_page {
        max_per_page
    } else {
        per_page
    };
    let offset = (page - 1) * per_page;
    (page, per_page, offset)
}

pub fn total_pages(total: i64, per_page: i32) -> i32 {
    ((total as f64) / (per_page as f64)).ceil() as i32
}

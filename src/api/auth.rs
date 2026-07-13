// SPDX-License-Identifier: GPL-3.0-only
use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::{header::AUTHORIZATION, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};

use crate::api::handlers::ApiHandlers;
use crate::api::response::ApiResponse;
use crate::config::read_config;

pub async fn require_api_key(
    State(handlers): State<Arc<ApiHandlers>>,
    request: Request,
    next: Next,
) -> Response {
    let config = read_config(&handlers.config);
    let Some(expected) = config
        .local_api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
    else {
        return next.run(request).await;
    };

    let authorized = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|provided| provided == expected);

    if authorized {
        next.run(request).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(ApiResponse::<()>::error(
                "Missing or invalid daemon API bearer token".to_string(),
            )),
        )
            .into_response()
    }
}

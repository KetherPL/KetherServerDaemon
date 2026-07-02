// SPDX-License-Identifier: GPL-3.0-only
use tracing::error;

use crate::api::error::ApiError;

pub fn classify_workshop_error(err: anyhow::Error) -> ApiError {
    let message = err.to_string();
    if message.contains("not found") {
        error!(error = %message, "Workshop update target not found");
        return ApiError::not_found(message);
    }
    error!(error = %message, "Workshop update failed");
    ApiError::internal(message)
}

pub fn classify_modify_error(err: anyhow::Error) -> ApiError {
    let message = err.to_string();
    if message.contains("not found") {
        error!(error = %message, "Map not found for modify");
        return ApiError::not_found(message);
    }
    if message.contains("Unknown or read-only field")
        || message.contains("Invalid source_kind")
        || message.contains("Invalid workshop_id")
    {
        error!(error = %message, "Invalid modify field or value");
        return ApiError::bad_request(message);
    }
    error!(error = %message, "Failed to modify map");
    ApiError::internal(err.to_string())
}

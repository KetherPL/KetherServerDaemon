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

pub fn classify_l4d2center_error(err: anyhow::Error) -> ApiError {
    let message = err.to_string();
    if message.contains("not found in L4D2Center catalog")
        || message.contains("not installed")
        || (message.contains("Map #") && message.contains("not found"))
    {
        error!(error = %message, "L4D2Center request target not found");
        return ApiError::not_found(message);
    }
    if message.contains("is not an L4d2Center map") {
        error!(error = %message, "L4D2Center update target invalid");
        return ApiError::bad_request(message);
    }
    error!(error = %message, "L4D2Center operation failed");
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
        || message.contains("Invalid installed_path")
        || message.contains("installed_path cannot be empty")
        || message.contains("file already exists")
        || message.contains("already used by map")
        || message.contains("Path contains parent directory reference")
    {
        error!(error = %message, "Invalid modify field or value");
        return ApiError::bad_request(message);
    }
    error!(error = %message, "Failed to modify map");
    ApiError::internal(err.to_string())
}

// SPDX-License-Identifier: GPL-3.0-only
use tracing::error;

use crate::api::error::ApiError;
use crate::api::types::{InstallMapRequest, ModifyMapRequest};

#[derive(Debug, Clone)]
pub enum InstallSource {
    Url(String),
    Workshop(u64),
}

pub fn parse_map_id(id: &str) -> Result<u64, ApiError> {
    id.parse::<u64>().map_err(|_| {
        error!(id = %id, "Invalid map ID format (expected integer)");
        ApiError::bad_request("Invalid map ID format (expected integer)")
    })
}

pub fn validate_optional_name(name: &Option<String>) -> Result<(), ApiError> {
    if let Some(name) = name
        && name.len() > 255
    {
        error!("Map name too long: {} characters", name.len());
        return Err(ApiError::bad_request(
            "Map name too long (max 255 characters)",
        ));
    }
    Ok(())
}

pub fn validate_install_request(req: &InstallMapRequest) -> Result<InstallSource, ApiError> {
    match (req.url.as_ref(), req.workshop_id) {
        (Some(_), Some(_)) => {
            error!("Both url and workshop_id provided, but only one is allowed");
            Err(ApiError::bad_request(
                "Both url and workshop_id provided, but only one is allowed",
            ))
        }
        (None, None) => {
            error!("Neither url nor workshop_id provided, one is required");
            Err(ApiError::bad_request(
                "Neither url nor workshop_id provided, one is required",
            ))
        }
        (Some(url), None) => {
            if url.len() > 2048 {
                error!("URL too long: {} characters", url.len());
                return Err(ApiError::bad_request("URL too long (max 2048 characters)"));
            }
            validate_optional_name(&req.name)?;
            Ok(InstallSource::Url(url.clone()))
        }
        (None, Some(workshop_id)) => {
            validate_optional_name(&req.name)?;
            Ok(InstallSource::Workshop(workshop_id))
        }
    }
}

pub fn validate_modify_request(req: &ModifyMapRequest) -> Result<(), ApiError> {
    if req.field.is_empty() {
        error!("Modify request missing field name");
        return Err(ApiError::bad_request("Modify request missing field name"));
    }

    if req.value.len() > 2048 {
        error!("Modify value too long: {} characters", req.value.len());
        return Err(ApiError::bad_request(
            "Modify value too long (max 2048 characters)",
        ));
    }

    Ok(())
}

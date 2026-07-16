// SPDX-License-Identifier: GPL-3.0-only
use chrono::{DateTime, TimeZone, Utc};
use std::time::Duration;
use steam_vent::{
    Connection, ConnectionTrait, ServerList,
};
use steam_vent_proto_steam::{
    steammessages_clientserver_ufs::{
        CMsgClientUFSGetUGCDetails, CMsgClientUFSGetUGCDetailsResponse,
    },
    steammessages_publishedfile_steamclient::{
        CPublishedFile_GetDetails_Request, CPublishedFile_GetDetails_Response,
    },
};
use tracing::{info, warn};

/// steam-vent defaults to 10s; UFS download-URL jobs often need longer.
const STEAM_JOB_TIMEOUT: Duration = Duration::from_secs(60);
const STEAM_DOWNLOAD_URL_RETRIES: u32 = 3;
const STEAM_CONNECT_RETRY_LIMIT: u32 = 3;
const STEAM_CONNECT_BACKOFF_BASE_SECS: u64 = 1;
const STEAM_CONNECT_BACKOFF_MAX_SECS: u64 = 30;

#[derive(thiserror::Error, Debug)]
pub enum SteamError {
    #[error("Failed to discover Steam servers: {0}")]
    ServerDiscovery(#[from] steam_vent::ServerDiscoveryError),
    
    #[error("Failed to connect to Steam: {0}")]
    Connection(#[from] steam_vent::ConnectionError),
    
    #[error("Network error: {0}")]
    Network(#[from] steam_vent::NetworkError),
    
    #[error("Workshop ID not found: {0}")]
    WorkshopIdNotFound(u64),
    
    #[error("Failed to get download URL: eresult={0}")]
    DownloadUrlFailed(i32),
    
    #[error("No download URL available for workshop item")]
    NoDownloadUrl,
}

impl SteamError {
    pub fn is_connection_error(&self) -> bool {
        matches!(
            self,
            Self::Connection(_)
                | Self::Network(
                    steam_vent::NetworkError::Ws(_)
                        | steam_vent::NetworkError::IO(_)
                        | steam_vent::NetworkError::EOF
                        | steam_vent::NetworkError::Timeout
                        | steam_vent::NetworkError::CryptoHandshakeFailed
                )
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkshopFileDetails {
    pub workshop_id: u64,
    pub hcontent: u64,
    pub time_updated: u32,
    pub file_size: u64,
    /// Direct CDN URL when available (preferred over UFS hcontent lookup).
    pub file_url: Option<String>,
}

fn parse_file_url(item: &steam_vent_proto_steam::steammessages_publishedfile_steamclient::PublishedFileDetails) -> Option<String> {
    if !item.has_file_url() {
        return None;
    }
    let url = item.file_url().trim().to_string();
    if url.is_empty() {
        None
    } else {
        Some(url)
    }
}

pub fn steam_time_to_utc(secs: u32) -> DateTime<Utc> {
    Utc.timestamp_opt(secs as i64, 0)
        .single()
        .unwrap_or_else(Utc::now)
}

fn is_steam_timeout(error: &SteamError) -> bool {
    matches!(
        error,
        SteamError::Network(steam_vent::NetworkError::Timeout)
    )
}

#[derive(Clone)]
pub struct SteamConnection {
    connection: Connection,
}

impl SteamConnection {
    /// Create a new Steam connection using anonymous authentication
    pub async fn new() -> Result<Self, SteamError> {
        info!("Discovering Steam servers");
        let server_list = ServerList::discover().await?;
        
        info!("Establishing anonymous Steam connection");
        let mut connection = Connection::anonymous(&server_list).await?;
        connection.set_timeout(STEAM_JOB_TIMEOUT);
        
        Ok(Self { connection })
    }

    /// Establish a Steam connection, retrying transient discovery and handshake failures.
    pub async fn connect_with_retry() -> Result<Self, SteamError> {
        for attempt in 1..=STEAM_CONNECT_RETRY_LIMIT {
            match Self::new().await {
                Ok(connection) => return Ok(connection),
                Err(error) if attempt < STEAM_CONNECT_RETRY_LIMIT => {
                    let exponential_delay =
                        STEAM_CONNECT_BACKOFF_BASE_SECS * (1_u64 << (attempt - 1));
                    let delay_secs = exponential_delay.min(STEAM_CONNECT_BACKOFF_MAX_SECS);
                    warn!(
                        error = %error,
                        attempt,
                        max_attempts = STEAM_CONNECT_RETRY_LIMIT,
                        delay_secs,
                        "Failed to establish Steam connection, retrying"
                    );
                    tokio::time::sleep(Duration::from_secs(delay_secs)).await;
                }
                Err(error) => return Err(error),
            }
        }

        unreachable!("Steam connection retry loop always returns")
    }

    /// Batch-fetch workshop file metadata from Steam.
    pub async fn get_workshop_file_details(
        &self,
        workshop_ids: &[u64],
    ) -> Result<Vec<WorkshopFileDetails>, SteamError> {
        if workshop_ids.is_empty() {
            return Ok(Vec::new());
        }

        info!(count = workshop_ids.len(), "Fetching workshop file details");

        let mut req = CPublishedFile_GetDetails_Request::new();
        req.publishedfileids = workshop_ids.to_vec();
        req.appid = Some(550); // Left 4 Dead 2 app ID

        let response: CPublishedFile_GetDetails_Response = self
            .connection
            .service_method(req)
            .await
            .map_err(SteamError::Network)?;

        let mut details = Vec::new();
        for item in &response.publishedfiledetails {
            let workshop_id = item.publishedfileid();
            let hcontent = item.hcontent_file();
            if workshop_id == 0 {
                continue;
            }
            let file_url = parse_file_url(item);
            // Keep CDN URL entries even when hcontent is missing (UFS fallback needs hcontent).
            if hcontent == 0 && file_url.is_none() {
                continue;
            }
            details.push(WorkshopFileDetails {
                workshop_id,
                hcontent,
                time_updated: item.time_updated(),
                file_size: item.file_size(),
                file_url,
            });
        }

        info!(returned = details.len(), "Got workshop file details");
        Ok(details)
    }
    
    /// Get download URL from hcontent handle, with retries on transient Steam timeouts.
    pub async fn get_download_url(&self, hcontent: u64) -> Result<String, SteamError> {
        let mut last_error = SteamError::Network(steam_vent::NetworkError::Timeout);

        for attempt in 1..=STEAM_DOWNLOAD_URL_RETRIES {
            match self.get_download_url_once(hcontent).await {
                Ok(url) => return Ok(url),
                Err(error) => {
                    last_error = error;
                    if is_steam_timeout(&last_error) && attempt < STEAM_DOWNLOAD_URL_RETRIES {
                        warn!(
                            hcontent,
                            attempt,
                            max_attempts = STEAM_DOWNLOAD_URL_RETRIES,
                            "Steam download URL request timed out, retrying"
                        );
                        tokio::time::sleep(Duration::from_secs(2 * attempt as u64)).await;
                        continue;
                    }
                    return Err(last_error);
                }
            }
        }

        Err(last_error)
    }

    async fn get_download_url_once(&self, hcontent: u64) -> Result<String, SteamError> {
        info!(hcontent, "Getting download URL from hcontent");
        
        let mut req = CMsgClientUFSGetUGCDetails::new();
        req.set_hcontent(hcontent);
        
        let response: CMsgClientUFSGetUGCDetailsResponse = self
            .connection
            .job::<CMsgClientUFSGetUGCDetails, CMsgClientUFSGetUGCDetailsResponse>(req)
            .await
            .map_err(SteamError::Network)?;
        
        let eresult = response.eresult();
        if eresult != 1 {
            return Err(SteamError::DownloadUrlFailed(eresult));
        }
        
        if !response.has_url() || response.url().is_empty() {
            return Err(SteamError::NoDownloadUrl);
        }
        
        let url = response.url();
        
        info!(hcontent, url = %url, "Got download URL");
        Ok(url.to_string())
    }
    
    /// Get connection reference for reuse
    pub fn connection(&self) -> &Connection {
        &self.connection
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[test]
    fn connection_errors_are_recoverable() {
        let errors = [
            SteamError::Connection(steam_vent::ConnectionError::Aborted),
            SteamError::Network(steam_vent::NetworkError::Ws(
                tokio_tungstenite::tungstenite::Error::AlreadyClosed,
            )),
            SteamError::Network(steam_vent::NetworkError::IO(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "connection reset",
            ))),
            SteamError::Network(steam_vent::NetworkError::EOF),
            SteamError::Network(steam_vent::NetworkError::Timeout),
            SteamError::Network(steam_vent::NetworkError::CryptoHandshakeFailed),
        ];

        for error in errors {
            assert!(
                error.is_connection_error(),
                "expected a recoverable connection error: {error}"
            );
        }
    }

    #[test]
    fn application_and_protocol_errors_are_not_connection_errors() {
        let errors = [
            SteamError::Network(steam_vent::NetworkError::InvalidHeader),
            SteamError::DownloadUrlFailed(2),
            SteamError::NoDownloadUrl,
        ];

        for error in errors {
            assert!(
                !error.is_connection_error(),
                "expected a non-connection error: {error}"
            );
        }
    }
}

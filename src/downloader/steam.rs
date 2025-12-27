// SPDX-License-Identifier: GPL-3.0-only
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
use tracing::info;

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

pub struct SteamConnection {
    connection: Connection,
}

impl SteamConnection {
    /// Create a new Steam connection using anonymous authentication
    pub async fn new() -> Result<Self, SteamError> {
        info!("Discovering Steam servers");
        let server_list = ServerList::discover().await?;
        
        info!("Establishing anonymous Steam connection");
        let connection = Connection::anonymous(&server_list).await?;
        
        Ok(Self { connection })
    }
    
    /// Get hcontent handle from a workshop ID
    pub async fn get_hcontent_from_workshop_id(
        &self,
        workshop_id: u64,
    ) -> Result<u64, SteamError> {
        info!(workshop_id, "Fetching workshop file details");
        
        let mut req = CPublishedFile_GetDetails_Request::new();
        req.publishedfileids = vec![workshop_id];
        req.appid = Some(550); // Left 4 Dead 2 app ID
        
        let response: CPublishedFile_GetDetails_Response = self
            .connection
            .service_method(req)
            .await
            .map_err(SteamError::Network)?;
        
        if response.publishedfiledetails.is_empty() {
            return Err(SteamError::WorkshopIdNotFound(workshop_id));
        }
        
        let details = &response.publishedfiledetails[0];
        let hcontent = details.hcontent_file();
        
        if hcontent == 0 {
            return Err(SteamError::WorkshopIdNotFound(workshop_id));
        }
        
        info!(workshop_id, hcontent, "Got hcontent from workshop ID");
        Ok(hcontent)
    }
    
    /// Get download URL from hcontent handle
    pub async fn get_download_url(&self, hcontent: u64) -> Result<String, SteamError> {
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


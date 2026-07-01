// SPDX-License-Identifier: GPL-3.0-only
pub mod traits;
pub mod client;
pub mod workshop;
pub mod zip;
pub mod steam;

#[cfg(test)]
mod test_lock;

pub use traits::Downloader;
pub use client::HttpClient;
pub use workshop::WorkshopDownloader;
pub use zip::ZipDownloader;
pub use steam::{SteamConnection, SteamError, WorkshopFileDetails, steam_time_to_utc};


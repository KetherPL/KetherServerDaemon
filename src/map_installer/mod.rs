// SPDX-License-Identifier: GPL-3.0-only
mod service;
mod url_parser;

pub use service::{
    needs_workshop_update, CompactReport, DiscoveryMode, DiscoveryReport,
    MapInstallationService, WorkshopUpdateAvailable, WorkshopUpdateReport,
};


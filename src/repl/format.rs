// SPDX-License-Identifier: GPL-3.0-only
use crate::catalog::{CatalogMapStatus, L4d2CenterCatalogEntry};
use crate::map_installer::{CompactReport, DiscoveryReport, L4d2CenterUpdateReport, WorkshopUpdateReport};
use crate::registry::{MapEntry, SourceKind};

pub fn source_kind_label(kind: SourceKind) -> &'static str {
    match kind {
        SourceKind::Workshop => "workshop",
        SourceKind::SirPlease => "sirplease",
        SourceKind::L4d2Center => "l4d2center",
        SourceKind::Other => "other",
    }
}

pub fn map_source_label(map: &MapEntry) -> String {
    match map.workshop_id {
        Some(id) => format!("workshop:{id}"),
        None => map.source_url.clone(),
    }
}

pub fn print_map_entry(map: &MapEntry) {
    let source_kind = source_kind_label(map.source_kind);
    let version = map.version.as_deref().unwrap_or("-");
    let source = map_source_label(map);

    println!(
        "  #{} | {} | version={} | source={} ({}) | path={}",
        map.id, map.name, version, source_kind, source, map.installed_path
    );
    if let Some(updated_at) = map.workshop_updated_at {
        println!("    workshop_updated_at: {updated_at}");
    }
}

pub fn print_map_detail(map: &MapEntry) {
    let source_kind = source_kind_label(map.source_kind);
    let workshop_id = map
        .workshop_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| "-".to_string());
    let version = map.version.as_deref().unwrap_or("-");
    let checksum = map.checksum.as_deref().unwrap_or("-");
    let checksum_kind = map.checksum_kind.as_deref().unwrap_or("-");

    println!("  id:             {}", map.id);
    println!("  name:           {}", map.name);
    println!("  source_kind:    {source_kind}");
    println!("  workshop_id:    {workshop_id}");
    println!("  source_url:     {}", map.source_url);
    println!("  version:        {version}");
    println!("  installed_path: {}", map.installed_path);
    println!("  installed_at:   {}", map.installed_at);
    if let Some(updated_at) = map.workshop_updated_at {
        println!("  workshop_updated_at: {updated_at}");
    }
    println!("  checksum:       {checksum}");
    println!("  checksum_kind:  {checksum_kind}");
}

pub fn print_compact_report(report: CompactReport) {
    let kept_count = report.kept.len();
    let id_range = if kept_count == 0 {
        "none".to_string()
    } else if kept_count == 1 {
        "1".to_string()
    } else {
        format!("1–{kept_count}")
    };

    println!(
        "Compact complete: {} orphaned record(s) removed, {} map(s) reindexed (IDs {id_range}).",
        report.removed.len(),
        kept_count
    );

    if !report.removed.is_empty() {
        println!("Removed orphaned records:");
        for map in &report.removed {
            print_map_entry(map);
        }
    }
}

pub fn print_discovery_report(report: DiscoveryReport) {
    println!(
        "Discovery complete: {} added, {} updated, {} already current, {} failed.",
        report.added.len(),
        report.updated.len(),
        report.skipped,
        report.failed
    );
    if report.added.is_empty() {
        println!("No new maps found.");
    } else {
        println!("Newly registered maps:");
        for map in report.added {
            print_map_entry(&map);
        }
    }
    if !report.updated.is_empty() {
        println!("Updated maps:");
        for map in report.updated {
            print_map_entry(&map);
        }
    }
}

pub fn print_workshop_check_report(report: &WorkshopUpdateReport) {
    println!(
        "Workshop check: {} update(s) available, {} up to date, {} failed, {} not workshop-linked",
        report.available.len(),
        report.skipped,
        report.failed.len(),
        report.not_workshop
    );

    for item in &report.available {
        let local = item
            .map
            .workshop_updated_at
            .map(|ts| ts.to_string())
            .unwrap_or_else(|| "-".to_string());
        println!(
            "  #{} | {} | workshop:{} | steam:{} | local:{}",
            item.map.id,
            item.map.name,
            item.workshop_id,
            item.steam_updated_at,
            local
        );
    }

    for failure in &report.failed {
        eprintln!("  Failed #{}: {}", failure.map_id, failure.error);
    }
}

pub fn print_workshop_update_report(report: &WorkshopUpdateReport) {
    println!(
        "Workshop update complete: {} updated, {} skipped, {} failed, {} not workshop-linked",
        report.updated.len(),
        report.skipped,
        report.failed.len(),
        report.not_workshop
    );

    for map in &report.updated {
        println!(
            "  Updated #{}: {} ({})",
            map.id, map.name, map.installed_path
        );
    }

    for failure in &report.failed {
        eprintln!("  Failed #{}: {}", failure.map_id, failure.error);
    }
}

fn catalog_status_label(status: &CatalogMapStatus) -> &'static str {
    match status {
        CatalogMapStatus::NotInstalled => "not_installed",
        CatalogMapStatus::UpToDate => "up_to_date",
        CatalogMapStatus::Outdated => "outdated",
    }
}

pub fn print_l4d2center_catalog(catalog: &[L4d2CenterCatalogEntry]) {
    if catalog.is_empty() {
        println!("L4D2Center catalog is empty.");
        return;
    }

    println!("L4D2Center catalog ({} map(s)):", catalog.len());
    for entry in catalog {
        let map_id = entry
            .map_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "-".to_string());
        println!(
            "  {} | status={} | map_id={} | md5={} | size={}",
            entry.name,
            catalog_status_label(&entry.status),
            map_id,
            entry.md5,
            entry.size
        );
    }
}

pub fn print_l4d2center_check_report(report: &L4d2CenterUpdateReport) {
    println!(
        "L4D2Center check: {} update(s) available, {} up to date, {} failed, {} not L4D2Center",
        report.available.len(),
        report.skipped,
        report.failed.len(),
        report.not_l4d2center
    );

    for item in &report.available {
        let local = item.local_md5.as_deref().unwrap_or("-");
        println!(
            "  #{} | {} | index:{} | local:{}",
            item.map_id, item.name, item.index_md5, local
        );
    }

    for failure in &report.failed {
        eprintln!("  Failed #{}: {}", failure.map_id, failure.error);
    }
}

pub fn print_l4d2center_update_report(report: &L4d2CenterUpdateReport) {
    println!(
        "L4D2Center update complete: {} updated, {} skipped, {} failed, {} not L4D2Center",
        report.updated.len(),
        report.skipped,
        report.failed.len(),
        report.not_l4d2center
    );

    for map in &report.updated {
        println!(
            "  Updated #{}: {} ({})",
            map.id, map.name, map.installed_path
        );
    }

    for failure in &report.failed {
        eprintln!("  Failed #{}: {}", failure.map_id, failure.error);
    }
}

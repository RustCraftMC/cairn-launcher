//! Static mod loader compatibility ranges for Minecraft versions.
//!
//! Used by the UI to hide unsupported loaders in installation menus without waiting for network requests.
//! Ranges are determined by release dates so new version naming schemes remain compatible.

use cairn_core::game::instance::LoaderKind;
use cairn_core::game::manifest::{ManifestEntry, VersionType};
pub use cairn_core::install::optifine::validate_loader_combination;

/// Earliest release date (`YYYY-MM-DD`) supported by the loader.
fn since(kind: LoaderKind) -> &'static str {
    match kind {
        // 18w43b
        LoaderKind::Fabric => "2018-10-24",
        // 1.14.4
        LoaderKind::Quilt => "2019-07-19",
        // 1.5.2
        LoaderKind::Forge => "2013-04-25",
        // 1.20.1
        LoaderKind::NeoForge => "2023-06-12",
        // 1.7.2
        LoaderKind::OptiFine => "2013-10-25",
        // 1.5.2
        LoaderKind::LiteLoader => "2013-04-25",
    }
}

pub fn loader_supported(kind: LoaderKind, mc: &ManifestEntry) -> bool {
    match mc.kind {
        VersionType::OldAlpha | VersionType::OldBeta => return false,
        // Only Fabric and Quilt support snapshot versions.
        VersionType::Snapshot if !matches!(kind, LoaderKind::Fabric | LoaderKind::Quilt) => return false,
        _ => {}
    }
    // Compare the leading 10 characters of the ISO 8601 release timestamp as a date string.
    mc.release_time.get(..10).is_some_and(|date| date >= since(kind))
}

pub use cairn_core::game::manifest::is_fool_version;

/// Returns true if at least one mod loader (excluding OptiFine) supports this game version.
pub fn any_loader_supported(mc: &ManifestEntry) -> bool {
    [LoaderKind::Fabric, LoaderKind::Quilt, LoaderKind::Forge, LoaderKind::NeoForge].into_iter().any(|k| loader_supported(k, mc))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: VersionType, time: &str) -> ManifestEntry {
        serde_json::from_value(serde_json::json!({
            "id": "x",
            "type": match kind {
                VersionType::Release => "release",
                VersionType::Snapshot => "snapshot",
                VersionType::OldBeta => "old_beta",
                VersionType::OldAlpha => "old_alpha",
                VersionType::Other => "other",
            },
            "url": "",
            "time": time,
            "releaseTime": time,
            "sha1": null
        }))
        .expect("清单条目")
    }

    #[test]
    fn ranges_follow_each_loaders_first_game_version() {
        let v1_7_10 = entry(VersionType::Release, "2014-06-26T00:00:00+00:00");
        assert!(loader_supported(LoaderKind::Forge, &v1_7_10));
        assert!(loader_supported(LoaderKind::OptiFine, &v1_7_10));
        assert!(!loader_supported(LoaderKind::Fabric, &v1_7_10));
        assert!(!loader_supported(LoaderKind::NeoForge, &v1_7_10));

        let v1_14_4 = entry(VersionType::Release, "2019-07-19T09:00:00+00:00");
        assert!(loader_supported(LoaderKind::Fabric, &v1_14_4));
        assert!(loader_supported(LoaderKind::Quilt, &v1_14_4));
        assert!(!loader_supported(LoaderKind::NeoForge, &v1_14_4));

        let v1_20_1 = entry(VersionType::Release, "2023-06-12T00:00:00+00:00");
        assert!(loader_supported(LoaderKind::NeoForge, &v1_20_1));

        // Year-based versions are matched by release date.
        let v26 = entry(VersionType::Release, "2026-09-01T00:00:00+00:00");
        assert!(
            [LoaderKind::Fabric, LoaderKind::Quilt, LoaderKind::Forge, LoaderKind::NeoForge, LoaderKind::OptiFine].iter().all(|k| loader_supported(*k, &v26))
        );
    }

    #[test]
    fn ancient_versions_have_no_loaders_and_snapshots_only_fabric_and_quilt() {
        let beta = entry(VersionType::OldBeta, "2011-03-01T00:00:00+00:00");
        let alpha = entry(VersionType::OldAlpha, "2010-12-01T00:00:00+00:00");
        assert!(!any_loader_supported(&beta));
        assert!(!any_loader_supported(&alpha));
        // Ancient version types are unsupported regardless of release date.
        assert!(!loader_supported(LoaderKind::Forge, &entry(VersionType::OldBeta, "2024-01-01T00:00:00+00:00")));

        let snapshot = entry(VersionType::Snapshot, "2024-05-01T00:00:00+00:00");
        assert!(loader_supported(LoaderKind::Fabric, &snapshot));
        assert!(loader_supported(LoaderKind::Quilt, &snapshot));
        assert!(!loader_supported(LoaderKind::Forge, &snapshot));
        assert!(!loader_supported(LoaderKind::NeoForge, &snapshot));
        assert!(!loader_supported(LoaderKind::OptiFine, &snapshot));
    }
}

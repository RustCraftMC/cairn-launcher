//! Loader picker rules: row states, the auto version pick and why an install is blocked.
//! The main loader is picked first; OptiFine only lists versions that fit it.
//! Pair rules live in [`validate_loader_combination`]. The UI and install request use it.

use cairn_bridge::support::validate_loader_combination;
use cairn_bridge::{LoaderKind, LoaderSpec, LoaderVersionInfo, Remote};
use cairn_i18n::tr;

use super::loaders::loader_name;
use crate::gui::{LoaderPick, VersionChoice};

#[derive(Debug, PartialEq)]
pub enum Row<'a> {
    Loading,
    Failed,
    /// No build of this loader exists for the game version.
    Unsupported,
    /// Conflicts with another selection, so it cannot be picked. The string says why.
    Blocked(String),
    Ready {
        /// Versions that fit the other choices, newest first.
        versions: Vec<&'a LoaderVersionInfo>,
        auto: Option<&'a LoaderVersionInfo>,
    },
}

#[derive(Debug)]
pub struct Plan<'a> {
    pub rows: Vec<(LoaderKind, Row<'a>)>,
    pub loader: Option<(LoaderKind, String)>,
    pub optifine: Option<String>,
    /// Why the install is blocked now. `None` means the loader part has no problem.
    pub problem: Option<String>,
}

impl Plan<'_> {
    pub fn row(&self, kind: LoaderKind) -> Option<&Row<'_>> {
        self.rows.iter().find(|(k, _)| *k == kind).map(|(_, r)| r)
    }

    pub fn spec(&self) -> Option<LoaderSpec> {
        let (kind, v) = self.loader.clone()?;
        match kind {
            LoaderKind::Fabric => Some(LoaderSpec::Fabric(Some(v))),
            LoaderKind::Quilt => Some(LoaderSpec::Quilt(Some(v))),
            LoaderKind::Forge => Some(LoaderSpec::Forge(Some(v))),
            LoaderKind::NeoForge => Some(LoaderSpec::NeoForge(Some(v))),
            _ => None,
        }
    }
}

pub const LOADERS: [LoaderKind; 4] = [LoaderKind::Fabric, LoaderKind::Quilt, LoaderKind::Forge, LoaderKind::NeoForge];

/// The recommended version, else the first stable one, else the first one.
fn best<'a>(list: &[&'a LoaderVersionInfo]) -> Option<&'a LoaderVersionInfo> {
    list.iter().find(|v| v.recommended).or_else(|| list.iter().find(|v| v.stable)).or(list.first()).copied()
}

pub fn forge_fits(mc: &str, of: &LoaderVersionInfo, forge: &str) -> bool {
    validate_loader_combination(mc, LoaderKind::Forge, of.requires.as_deref(), Some(forge)).is_ok()
}

/// OptiFine conflict with the loader kind, judged by game version only.
pub fn kind_conflict(mc: &str, kind: LoaderKind) -> Option<String> {
    // An empty Forge version means no Forge limit.
    // Only the game version and loader kind are checked here.
    validate_loader_combination(mc, kind, Some(""), None).err().map(|e| e.to_string())
}

pub fn plan<'a>(mc: &str, pick: &LoaderPick, lists: impl Fn(LoaderKind) -> Option<&'a Remote<Vec<LoaderVersionInfo>>>) -> Plan<'a> {
    let ready = |kind| match lists(kind) {
        Some(Remote::Ready(list)) => Some(list.as_slice()),
        _ => None,
    };
    let with_optifine = pick.optifine.is_some();
    let forge_list = ready(LoaderKind::Forge).unwrap_or(&[]);
    let of_list = ready(LoaderKind::OptiFine).unwrap_or(&[]);
    let forge_exact = (pick.loader == Some(LoaderKind::Forge)).then(|| pick.loader_version.exact()).flatten();
    let of_exact = pick.optifine.as_ref().and_then(VersionChoice::exact);
    let of_exact_info = of_exact.and_then(|v| of_list.iter().find(|x| x.version == v));

    // OptiFine lists only builds that pair with the chosen Forge.
    // With Forge on auto, list builds that pair with at least one Forge.
    let of_ok = |of: &LoaderVersionInfo| match (pick.loader, forge_exact) {
        (Some(LoaderKind::Forge), Some(f)) => forge_fits(mc, of, f),
        (Some(LoaderKind::Forge), None) => forge_list.is_empty() || forge_list.iter().any(|f| forge_fits(mc, of, &f.version)),
        _ => true,
    };

    let row_of = |kind: LoaderKind, blocked: Option<String>, keep: &dyn Fn(&LoaderVersionInfo) -> bool| -> Row<'a> {
        match lists(kind) {
            None | Some(Remote::Idle | Remote::Loading) => Row::Loading,
            Some(Remote::Failed(_)) => Row::Failed,
            Some(Remote::Ready(list)) if list.is_empty() => Row::Unsupported,
            Some(Remote::Ready(_)) if blocked.is_some() => Row::Blocked(blocked.unwrap_or_default()),
            Some(Remote::Ready(list)) => {
                let versions: Vec<&LoaderVersionInfo> = list.iter().filter(|v| keep(v)).collect();
                let auto = best(&versions);
                Row::Ready { versions, auto }
            }
        }
    };

    let mut rows = Vec::new();
    for kind in LOADERS {
        rows.push((kind, row_of(kind, None, &|_| true)));
    }
    let of_blocked = pick.loader.and_then(|k| kind_conflict(mc, k));
    rows.push((LoaderKind::OptiFine, row_of(LoaderKind::OptiFine, of_blocked, &of_ok)));

    // With Forge on auto and OptiFine installed, pick OptiFine first (among builds with a Forge).
    // Then take the Forge from its annotation, so auto never lands on an unpaired Forge.
    let mut auto_forge_for_of: Option<&'a LoaderVersionInfo> = None;
    if pick.loader == Some(LoaderKind::Forge) && forge_exact.is_none() && with_optifine {
        let of = match (rows.iter().find(|(k, _)| *k == LoaderKind::OptiFine), of_exact_info) {
            (_, Some(of)) => Some(of),
            (Some((_, Row::Ready { auto, .. })), None) => *auto,
            _ => None,
        };
        if let (Some(of), Some((_, Row::Ready { versions, .. }))) = (of, rows.iter().find(|(k, _)| *k == LoaderKind::Forge)) {
            let fitting: Vec<_> = versions.iter().copied().filter(|f| forge_fits(mc, of, &f.version)).collect();
            auto_forge_for_of = best(&fitting);
        }
        if let Some((_, Row::Ready { auto, .. })) = rows.iter_mut().find(|(k, _)| *k == LoaderKind::Forge) {
            *auto = auto_forge_for_of;
        }
    }

    let mut problem = None;
    let mut resolve = |kind: LoaderKind, choice: &VersionChoice, rows: &[(LoaderKind, Row<'a>)]| -> Option<String> {
        let name = loader_name(kind);
        let row = rows.iter().find(|(k, _)| *k == kind).map(|(_, r)| r);
        let found = match row {
            Some(Row::Ready { versions, auto }) => match choice {
                VersionChoice::Auto => auto.map(|v| v.version.clone()).ok_or_else(|| tr!("loaders.no_compatible_named", name = name)),
                VersionChoice::Exact(v) => {
                    versions.iter().find(|x| &x.version == v).map(|x| x.version.clone()).ok_or_else(|| tr!("loaders.incompatible_named", name = name, v = v))
                }
            },
            Some(Row::Blocked(reason)) => Err(reason.clone()),
            Some(Row::Unsupported) => Err(tr!("loaders.unsupported_game_version_named", name = name)),
            Some(Row::Failed) => Err(tr!("loaders.load_named_failed", name = name, error = "")),
            Some(Row::Loading) | None => Err(tr!("loaders.loading_named", name = name)),
        };
        match found {
            Ok(v) => Some(v),
            Err(e) => {
                problem.get_or_insert(e);
                None
            }
        }
    };
    let loader = pick.loader.and_then(|k| resolve(k, &pick.loader_version, &rows).map(|v| (k, v)));
    let optifine = pick.optifine.as_ref().and_then(|c| resolve(LoaderKind::OptiFine, c, &rows));

    Plan { rows, loader, optifine, problem }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(version: &str, recommended: bool, requires: Option<&str>) -> LoaderVersionInfo {
        LoaderVersionInfo { version: version.into(), stable: true, recommended, requires: requires.map(Into::into) }
    }

    struct Lists(Vec<(LoaderKind, Remote<Vec<LoaderVersionInfo>>)>);

    impl Lists {
        fn get<'a>(&'a self) -> impl Fn(LoaderKind) -> Option<&'a Remote<Vec<LoaderVersionInfo>>> + 'a {
            |k| self.0.iter().find(|(x, _)| *x == k).map(|(_, r)| r)
        }
    }

    fn lists_1201() -> Lists {
        Lists(vec![
            (LoaderKind::Fabric, Remote::Ready(vec![v("0.20.1", true, None), v("0.16.14", false, None)])),
            (LoaderKind::Quilt, Remote::Ready(vec![v("0.30.1", true, None)])),
            (LoaderKind::Forge, Remote::Ready(vec![v("47.4.0", true, None), v("47.2.18", false, None), v("47.0.35", false, None)])),
            (LoaderKind::NeoForge, Remote::Ready(vec![v("47.1.106", true, None)])),
            (LoaderKind::OptiFine, Remote::Ready(vec![v("HD_U_I6", true, Some("Forge 47.2.18")), v("HD_U_I5", false, Some("Forge 47.0.35"))])),
        ])
    }

    fn pick(loader: Option<LoaderKind>, version: VersionChoice, optifine: Option<VersionChoice>) -> LoaderPick {
        LoaderPick { loader, loader_version: version, optifine, ..LoaderPick::default() }
    }

    #[test]
    fn auto_resolves_to_the_shown_version() {
        let l = lists_1201();
        let p = plan("1.20.1", &pick(Some(LoaderKind::Fabric), VersionChoice::Auto, None), l.get());
        assert_eq!(p.loader, Some((LoaderKind::Fabric, "0.20.1".into())));
        assert!(p.problem.is_none());
        assert!(matches!(p.spec(), Some(LoaderSpec::Fabric(Some(v))) if v == "0.20.1"));
    }

    #[test]
    fn forge_and_optifine_both_auto_pick_a_matching_pair() {
        let l = lists_1201();
        let p = plan("1.20.1", &pick(Some(LoaderKind::Forge), VersionChoice::Auto, Some(VersionChoice::Auto)), l.get());
        assert_eq!(p.optifine.as_deref(), Some("HD_U_I6"));
        assert_eq!(p.loader, Some((LoaderKind::Forge, "47.2.18".into())));
        // The main loader is not limited by the add-on.
        // 47.4.0 is listed even without an OptiFine build.
        let Some(Row::Ready { versions, .. }) = p.row(LoaderKind::Forge) else { panic!() };
        assert!(versions.iter().any(|f| f.version == "47.4.0"));
    }

    #[test]
    fn optifine_follows_the_chosen_forge() {
        let l = lists_1201();
        let p = plan("1.20.1", &pick(Some(LoaderKind::Forge), VersionChoice::Exact("47.0.35".into()), Some(VersionChoice::Auto)), l.get());
        assert_eq!(p.optifine.as_deref(), Some("HD_U_I5"));
        let Some(Row::Ready { versions, .. }) = p.row(LoaderKind::OptiFine) else { panic!() };
        assert_eq!(versions.iter().map(|f| f.version.as_str()).collect::<Vec<_>>(), ["HD_U_I5"]);
        let p = plan("1.20.1", &pick(Some(LoaderKind::Forge), VersionChoice::Auto, Some(VersionChoice::Exact("HD_U_I5".into()))), l.get());
        assert_eq!(p.loader, Some((LoaderKind::Forge, "47.0.35".into())));
        // Forge with no OptiFine build is picked. The OptiFine row explains why.
        let p = plan("1.20.1", &pick(Some(LoaderKind::Forge), VersionChoice::Exact("47.4.0".into()), Some(VersionChoice::Auto)), l.get());
        assert!(p.problem.is_some());
    }

    #[test]
    fn optifine_is_blocked_by_an_incompatible_main_loader() {
        let l = lists_1201();
        let p = plan("1.20.1", &pick(Some(LoaderKind::NeoForge), VersionChoice::Auto, None), l.get());
        assert!(matches!(p.row(LoaderKind::OptiFine), Some(Row::Blocked(r)) if r.contains("NeoForge")));
        let p = plan("1.20.5", &pick(Some(LoaderKind::Fabric), VersionChoice::Auto, None), l.get());
        assert!(matches!(p.row(LoaderKind::OptiFine), Some(Row::Blocked(_))));
        let p = plan("1.20.1", &pick(None, VersionChoice::Auto, None), l.get());
        assert!(matches!(p.row(LoaderKind::OptiFine), Some(Row::Ready { .. })));
        // The main loader row is never blocked by the add-on.
        let p = plan("1.20.1", &pick(None, VersionChoice::Auto, Some(VersionChoice::Auto)), l.get());
        assert!(LOADERS.iter().all(|k| matches!(p.row(*k), Some(Row::Ready { .. }))));
    }

    #[test]
    fn fabric_loader_version_is_not_restricted_with_optifine() {
        let l = lists_1201();
        let p = plan("1.20.1", &pick(Some(LoaderKind::Fabric), VersionChoice::Exact("0.20.1".into()), Some(VersionChoice::Auto)), l.get());
        assert!(p.problem.is_none());
        assert_eq!(p.optifine.as_deref(), Some("HD_U_I6"));
    }

    #[test]
    fn loading_and_failed_lists_explain_why_install_is_blocked() {
        let mut l = lists_1201();
        l.0[2].1 = Remote::Failed("timeout".into());
        let p = plan("1.20.1", &pick(Some(LoaderKind::Forge), VersionChoice::Auto, None), l.get());
        assert_eq!(p.row(LoaderKind::Forge), Some(&Row::Failed));
        assert_eq!(p.problem.as_deref(), Some(cairn_i18n::tr!("loaders.load_named_failed", name = "Forge", error = "")).as_deref());
        l.0[2].1 = Remote::Loading;
        let p = plan("1.20.1", &pick(Some(LoaderKind::Forge), VersionChoice::Auto, None), l.get());
        assert_eq!(p.problem.as_deref(), Some(cairn_i18n::tr!("loaders.loading_named", name = "Forge")).as_deref());
    }

    #[test]
    fn optifine_without_forge_annotation_cannot_pair_with_forge() {
        let l = Lists(vec![
            (LoaderKind::Forge, Remote::Ready(vec![v("14.23.5.2860", true, None)])),
            (LoaderKind::OptiFine, Remote::Ready(vec![v("HD_U_G5", true, None)])),
        ]);
        let p = plan("1.12.2", &pick(Some(LoaderKind::Forge), VersionChoice::Auto, Some(VersionChoice::Auto)), l.get());
        assert!(p.problem.is_some());
    }
}

//! Derives the Java version range an instance can use.
//! Rules come from the version JSON's `javaVersion`, the release time, and known loader
//! compatibility issues, collected from community launchers.

use serde::Serialize;

use crate::game::instance::{LoaderInfo, LoaderKind};
use crate::util::version::compare_loose;

type Ver = (u32, u32, u32);

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct JavaRequirement {
    /// Inclusive lower bound.
    pub min: Option<Ver>,
    /// Exclusive upper bound.
    pub max: Option<Ver>,
    pub json_major: Option<u32>,
    /// Mojang runtime component name, e.g. `java-runtime-delta`.
    pub component: Option<String>,
}

impl JavaRequirement {
    pub fn accepts(&self, v: Ver) -> bool {
        self.min.is_none_or(|m| v >= m) && self.max.is_none_or(|m| v < m)
    }

    fn at_least(&mut self, v: Ver) {
        self.min = Some(self.min.map_or(v, |m| m.max(v)));
    }

    fn below(&mut self, v: Ver) {
        self.max = Some(self.max.map_or(v, |m| m.min(v)));
    }

    /// Preferred major: the version JSON's major if set, otherwise the major of the lower bound.
    pub fn preferred_major(&self) -> Option<u32> {
        let p = self.json_major.or(self.min.map(|m| m.0));
        match (p, self.max) {
            (Some(p), Some(max)) if (p, 0, 0) >= max => Some(max.0.saturating_sub(1).max(8)),
            (None, Some(max)) => Some(if max.0 <= 9 { 8 } else { max.0 - 1 }),
            (p, _) => p,
        }
    }

    pub fn is_satisfiable(&self) -> bool {
        match (self.min, self.max) {
            (Some(a), Some(b)) => a < b,
            _ => true,
        }
    }

    pub fn describe(&self) -> String {
        let f = |v: Ver| if v.1 == 0 && v.2 == 0 { v.0.to_string() } else { format!("{}.{}.{}", v.0, v.1, v.2) };
        match (self.min, self.max) {
            (Some(a), Some(b)) => format!("Java {} 及以上、{} 以下", f(a), f(b)),
            (Some(a), None) => format!("Java {} 及以上", f(a)),
            (None, Some(b)) => format!("Java {} 以下", f(b)),
            (None, None) => "任意 Java".to_string(),
        }
    }
}

/// Comparable form of an MC version, e.g. `1.20.5` to (1, 20, 5) and year-based `26.1` to
/// (26, 1, 0).
fn mc_tuple(v: &str) -> Option<Ver> {
    let mut it = v.split(['.', '-', ' ']).map(|s| s.parse::<u32>());
    let a = it.next()?.ok()?;
    let b = it.next().and_then(Result::ok).unwrap_or(0);
    let c = it.next().and_then(Result::ok).unwrap_or(0);
    Some((a, b, c))
}

pub struct RequirementInput<'a> {
    pub mc_version: Option<&'a str>,
    pub release_time: Option<&'a str>,
    pub json_major: Option<u32>,
    pub component: Option<&'a str>,
    pub loaders: &'a [LoaderInfo],
}

pub fn derive(input: &RequirementInput<'_>) -> JavaRequirement {
    let mut r = JavaRequirement { json_major: input.json_major, component: input.component.map(str::to_string), ..Default::default() };
    let mc = input.mc_version.and_then(mc_tuple);
    let date = input.release_time.map(crate::util::date_part).unwrap_or("");
    let ge = |v: Ver| mc.is_some_and(|m| m >= v);
    let le = |v: Ver| mc.is_some_and(|m| m <= v);
    let known = mc.is_some();

    // Without a parseable MC version, the release date decides the baseline.
    if ge((1, 20, 5)) || (!known && date >= "2024-04-02") {
        r.at_least((21, 0, 0));
    } else if ge((1, 18, 0)) || (!known && date >= "2021-11-16") {
        r.at_least((17, 0, 0));
    } else if ge((1, 17, 0)) || (!known && date >= "2021-05-11") {
        r.at_least((16, 0, 0));
    } else if date >= "2017" {
        r.at_least((8, 0, 0));
    } else if !date.is_empty() && date <= "2013-05-01" {
        r.below((9, 0, 0));
    }
    if let Some(m) = input.json_major {
        r.at_least((m, 0, 0));
    }

    let loader = |k: LoaderKind| input.loaders.iter().find(|l| l.kind == k).map(|l| l.version.as_str());
    let vge = |v: &str, bound: &str| compare_loose(v, bound).is_ge();
    let vle = |v: &str, bound: &str| compare_loose(v, bound).is_le();

    if loader(LoaderKind::OptiFine).is_some() && known {
        if le((1, 6, 99)) {
            r.below((9, 0, 0));
        } else if ge((1, 8, 0)) && le((1, 12, 99)) {
            r.at_least((8, 0, 0));
            r.below((9, 0, 0));
        }
    }

    if let Some(f) = loader(LoaderKind::Forge) {
        if ge((1, 6, 1)) && le((1, 7, 2)) {
            r.at_least((7, 0, 0));
            r.below((8, 0, 0));
        } else if !known || le((1, 12, 99)) {
            r.below((9, 0, 0));
        } else if le((1, 14, 99)) {
            r.at_least((8, 0, 0));
            r.below((11, 0, 0));
        } else if le((1, 15, 99)) {
            r.at_least((8, 0, 0));
            r.below((16, 0, 0));
        } else if vge(f, "34.0.0") && vle(f, "36.2.25") {
            // Early 1.16.x Forge does not work with Java 8 builds after 8u321.
            r.below((8, 0, 321));
        } else if vge(f, "36.2.26") && vle(f, "36.999") {
            r.below((24, 0, 0));
        } else if vge(f, "37.0.0") && vle(f, "37.0.79") {
            r.below((17, 0, 0));
        } else if ge((1, 18, 0)) && le((1, 18, 99)) && loader(LoaderKind::OptiFine).is_some() {
            r.below((19, 0, 0));
        } else if vge(f, "45.0.21") && vle(f, "45.0.65") {
            r.below((20, 0, 0));
        } else if vge(f, "45.0.66") && vle(f, "47.4.8") {
            r.below((22, 0, 0));
        }
    }

    // The 1.20.1 NeoForge branch and early 20.2 builds do not support Java 22+.
    if let Some(n) = loader(LoaderKind::NeoForge)
        && (mc == Some((1, 20, 1)) || vle(n, "20.2.62-beta"))
    {
        r.below((22, 0, 0));
    }

    if let Some(fl) = loader(LoaderKind::Fabric) {
        if ge((1, 15, 0)) && le((1, 16, 99)) {
            r.at_least((8, 0, 0));
        } else if ge((1, 18, 0)) {
            r.at_least((17, 0, 0));
        }
        if compare_loose(fl, "0.17.0").is_lt() {
            r.below((25, 0, 0));
        }
    }

    if loader(LoaderKind::LiteLoader).is_some() {
        r.below((9, 0, 0));
    }

    // If min exceeds max, drop min: loader limits are more trustworthy
    // than the vanilla baseline.
    if !r.is_satisfiable() {
        r.min = None;
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input<'a>(mc: &'a str, date: &'a str, json: Option<u32>, loaders: &'a [LoaderInfo]) -> RequirementInput<'a> {
        RequirementInput { mc_version: Some(mc), release_time: Some(date), json_major: json, component: None, loaders }
    }

    #[test]
    fn vanilla_baselines() {
        let r = derive(&input("1.20.6", "2024-04-29T12:00:00+00:00", Some(21), &[]));
        assert!(r.accepts((21, 0, 3)) && !r.accepts((17, 0, 9)));
        let r = derive(&input("1.12.2", "2017-09-18T08:39:46+00:00", Some(8), &[]));
        assert!(r.accepts((8, 0, 392)));
        let r = derive(&input("1.5.2", "2013-04-25T15:45:00+00:00", None, &[]));
        assert!(!r.accepts((17, 0, 0)) && r.accepts((8, 0, 0)));
    }

    #[test]
    fn forge_1_12_needs_java_8() {
        let l = [LoaderInfo { kind: LoaderKind::Forge, version: "14.23.5.2860".into() }];
        let r = derive(&input("1.12.2", "2017-09-18T08:39:46+00:00", Some(8), &l));
        assert!(r.accepts((8, 0, 392)) && !r.accepts((17, 0, 0)));
        assert_eq!(r.preferred_major(), Some(8));
    }

    #[test]
    fn year_versions_compare_above_classic() {
        let r = derive(&input("26.1", "2026-03-24T00:00:00+00:00", Some(25), &[]));
        assert!(r.accepts((25, 0, 1)) && !r.accepts((21, 0, 5)));
    }
}

//! Crashes and launch failures: records the analysis, tells the user, and exports error reports.

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use cairn_core::crash::{CrashAnalysis, explain_error};
use cairn_core::util::fsx::sanitize_file_name;
use cairn_i18n::tr;

use crate::{Bridge, CrashInfo, CrashKind, MAX_CRASHES, Model, ToastAction, ToastKind, dialogs, local_now};

impl Bridge {
    /// Called when the analysis of a game crash arrives (`Event::GameCrashed`).
    pub(crate) fn record_game_crash(&self, m: &mut Model, game: u64, instance: String, code: Option<i32>, run_secs: u64, analysis: CrashAnalysis) {
        let text = tr!("crash.toast_game", name = instance_name(m, &instance));
        self.push_crash(m, CrashKind::Game { game, code, run_secs }, instance, analysis, text);
    }

    pub(crate) fn record_launch_failure(&self, m: &mut Model, task: u64, instance: &str, err: &cairn_core::Error) {
        let text = tr!("crash.toast_launch", name = instance_name(m, instance));
        self.push_crash(m, CrashKind::Launch { task, error: err.to_string() }, instance.to_string(), explain_error(err), text);
    }

    fn push_crash(&self, m: &mut Model, kind: CrashKind, instance: String, analysis: CrashAnalysis, text: String) {
        let id = self.inner.next_crash.fetch_add(1, Ordering::Relaxed);
        m.crashes.push(CrashInfo { id, kind, instance, analysis });
        if m.crashes.len() > MAX_CRASHES {
            let drop = m.crashes.len() - MAX_CRASHES;
            m.crashes.drain(..drop);
        }
        self.toast_with(m, ToastKind::Error, text, Some(ToastAction::ShowCrash(id)));
    }

    /// Exports an error report after a save dialog.
    /// `summary` is written as it is: it is the text the crash panel shows.
    pub fn export_crash(&self, m: &mut Model, id: u64, summary: String) {
        let Some(l) = self.ready_launcher(m) else { return };
        let Some(info) = m.crash(id).cloned() else { return };
        if self.inner.dialog_open.swap(true, Ordering::SeqCst) {
            return;
        }
        let default = default_report_path(&instance_name(m, &info.instance));
        let b = self.clone();
        self.inner.rt.spawn(async move {
            let picked = tokio::task::spawn_blocking(move || {
                dialogs::save_file_dialog(tr!("crash.save_title"), &default.display().to_string(), &[(tr!("crash.save_filter"), "*.zip")])
            })
            .await
            .ok()
            .flatten();
            b.inner.dialog_open.store(false, Ordering::SeqCst);
            let Some(dest) = picked else { return };
            let game = match info.kind {
                CrashKind::Game { game, .. } => Some(game),
                CrashKind::Launch { .. } => None,
            };
            match l.export_crash_report(game, &info.instance, summary, dest.clone()).await {
                Ok(()) => b.post(move |b, m| {
                    b.toast(m, ToastKind::Success, tr!("crash.exported"));
                    b.open_path(&dest);
                }),
                Err(e) => {
                    let msg = tr!("crash.export_failed", error = e);
                    b.post(move |b, m| b.toast(m, ToastKind::Error, msg))
                }
            };
        });
    }
}

fn instance_name(m: &Model, id: &str) -> String {
    m.instance(id).map(|i| i.settings.display_name.clone().unwrap_or_else(|| i.name.clone())).unwrap_or_else(|| id.to_string())
}

fn default_report_path(instance: &str) -> PathBuf {
    let t = local_now();
    let name = format!("{}-{}-{:04}{:02}{:02}-{:02}{:02}{:02}.zip", tr!("crash.report_file"), instance, t.year, t.month, t.day, t.hour, t.minute, t.second);
    let name = sanitize_file_name(&name);
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from);
    let desktop = home.iter().flat_map(|h| [h.join("Desktop"), h.join("OneDrive").join("Desktop")]).find(|d| d.is_dir());
    match desktop {
        Some(d) => d.join(name),
        None => PathBuf::from(name),
    }
}

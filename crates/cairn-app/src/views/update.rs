//! Update panel: shows new version details, changelog, download progress, and restart action.

use cairn_bridge::UpdateStatus;
use cairn_i18n::tr;
use cairn_ui::{
    Align, El, Icon, Insets, Justify, Styled, button, column, icon, label, progress_bar, row, scroll,
};

use super::{empty_state, frame, header, section};
use crate::gui::Gui;
use crate::style::{GREEN, RED, accent, fg};

pub fn view(g: &Gui) -> El<Gui> {
    let lead = row()
        .gap(10.0)
        .align(Align::Center)
        .child(icon(Icon::Download).icon_size(20.0).color(accent()))
        .child(label(tr!("update.title")).title());

    let Some(info) = g.m.update_info.as_ref().or_else(|| match &g.m.update_status {
        UpdateStatus::Available(i) => Some(i),
        _ => None,
    }) else {
        return frame(
            header(lead, Vec::new()),
            empty_state(Icon::Check, tr!("settings.up_to_date"), tr!("settings.auto_check_desc")),
            None,
        );
    };

    let mut body_items: Vec<El<Gui>> = Vec::new();

    // 1. Version comparison card
    let version_badge = row()
        .fill_width()
        .align(Align::Center)
        .gap(12.0)
        .padding(Insets::new(14.0, 16.0, 14.0, 16.0))
        .background(accent().with_opacity(0.08))
        .corner_radius(10.0)
        .child(
            column()
                .gap(2.0)
                .child(label(tr!("update.current_version")).caption().secondary())
                .child(label(format!("v{}", info.current_version)).weight(700)),
        )
        .child(icon(Icon::ChevronRight).icon_size(16.0).color(fg(0.4)))
        .child(
            column()
                .gap(2.0)
                .child(label(tr!("update.latest_version")).caption().color(accent()))
                .child(label(format!("v{}", info.target_version)).weight(700).color(accent())),
        )
        .child(row().grow(1.0).min_width(0.0))
        .child(
            label(format!("{:.1} MiB", info.size as f64 / 1048576.0))
                .caption()
                .secondary(),
        );
    body_items.push(version_badge.into());

    // 2. Changelog / Release notes
    body_items.push(section(tr!("update.changelog"), Icon::Info));

    let changelog_lines: Vec<El<Gui>> = info
        .changelog
        .lines()
        .map(|l| label(l).fill_width().into())
        .collect();

    let changelog_box = column()
        .fill_width()
        .gap(6.0)
        .padding(Insets::new(12.0, 14.0, 12.0, 14.0))
        .background(fg(0.04))
        .corner_radius(10.0)
        .children(changelog_lines);
    body_items.push(changelog_box.into());

    // 3. Status indication (if downloading, ready to restart, or error)
    match &g.m.update_status {
        UpdateStatus::Downloading { progress, downloaded, total } => {
            let percent = (progress * 100.0) as u32;
            let dl_text = format!("{:.1} / {:.1} MiB ({percent}%)", *downloaded as f64 / 1048576.0, *total as f64 / 1048576.0);
            let dl_box = column()
                .fill_width()
                .gap(8.0)
                .padding(Insets::new(8.0, 0.0, 4.0, 0.0))
                .child(
                    row()
                        .fill_width()
                        .justify(Justify::SpaceBetween)
                        .child(label(tr!("settings.downloading_update").replace("{percent}", &percent.to_string())).weight(700))
                        .child(label(dl_text).caption().secondary()),
                )
                .child(progress_bar(Some(*progress)).fill_width());
            body_items.push(dl_box.into());
        }
        UpdateStatus::ReadyToRestart => {
            let ready_box = row()
                .fill_width()
                .gap(8.0)
                .align(Align::Center)
                .padding(Insets::new(8.0, 12.0, 8.0, 12.0))
                .background(GREEN.with_opacity(0.1))
                .corner_radius(8.0)
                .child(icon(Icon::Check).icon_size(16.0).color(GREEN))
                .child(label(tr!("update.ready_desc")).color(GREEN).weight(600));
            body_items.push(ready_box.into());
        }
        UpdateStatus::Failed(err) => {
            let err_box = row()
                .fill_width()
                .gap(8.0)
                .align(Align::Center)
                .padding(Insets::new(8.0, 12.0, 8.0, 12.0))
                .background(RED.with_opacity(0.1))
                .corner_radius(8.0)
                .child(icon(Icon::Warning).icon_size(16.0).color(RED))
                .child(label(err).color(RED));
            body_items.push(err_box.into());
        }
        _ => {}
    }

    let body = scroll()
        .grow(1.0)
        .min_height(0.0)
        .align(Align::Stretch)
        .padding(Insets::new(12.0, 18.0, 18.0, 18.0))
        .gap(12.0)
        .children(body_items);

    // 4. Footer actions
    let action_btn = match &g.m.update_status {
        UpdateStatus::Downloading { progress, .. } => {
            let percent = (progress * 100.0) as u32;
            button(format!("{percent}%")).icon(Icon::Download).enabled(false)
        }
        UpdateStatus::ReadyToRestart => {
            button(tr!("settings.restart_btn")).accent().icon(Icon::Refresh).on_click(|g: &mut Gui| {
                g.bridge.apply_launcher_update(&mut g.m);
            })
        }
        UpdateStatus::Failed(_) => {
            button(tr!("common.retry")).accent().icon(Icon::Refresh).on_click(|g: &mut Gui| {
                g.bridge.download_update(&mut g.m);
            })
        }
        _ => {
            button(tr!("update.download_now")).accent().icon(Icon::Download).on_click(|g: &mut Gui| {
                g.bridge.download_update(&mut g.m);
            })
        }
    };

    let footer = row()
        .fill_width()
        .padding(Insets::new(10.0, 14.0, 10.0, 14.0))
        .gap(8.0)
        .justify(Justify::End)
        .child(button(tr!("update.later")).on_click(|g: &mut Gui| g.close()))
        .child(action_btn);

    frame(header(lead, Vec::new()), body.into(), Some(footer.into()))
}

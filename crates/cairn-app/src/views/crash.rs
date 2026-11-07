//! Crash panel, shown after a game crash or a launch failure.
//! Lists the analyzed causes and advice, with the key log expandable.
//! The bottom bar copies the analysis, opens the game folder and exports an error report.

use cairn_bridge::{CrashInfo, CrashKind, Finding, format_exit_code};
use cairn_i18n::tr;
use cairn_ui::{Align, El, Icon, Insets, Justify, Styled, button, collapse, column, icon, label, list_item, row, scroll};

use super::{empty_state, format, frame, header};
use crate::gui::Gui;
use crate::style::{AMBER, RED, fg};

pub fn view(g: &Gui, id: u64) -> El<Gui> {
    let Some(info) = g.m.crash(id) else {
        let lead = label(tr!("crash.title_game")).title();
        return frame(header(lead, Vec::new()), empty_state(Icon::Check, tr!("crash.unknown_title"), ""), None);
    };
    let title = if info.is_launch_failure() { tr!("crash.title_launch") } else { tr!("crash.title_game") };
    let lead = row().gap(10.0).child(icon(Icon::Warning).icon_size(18.0).color(RED)).child(label(title).title());

    let mut rows: Vec<El<Gui>> = vec![label(meta_line(g, info)).caption().into()];
    if info.analysis.findings.is_empty() {
        rows.push(card(Icon::Info, fg(0.7), tr!("crash.unknown_title"), tr!("crash.unknown_advice"), &[]));
    }
    for f in &info.analysis.findings {
        let (title, advice) = cause_text(f);
        rows.push(card(Icon::Warning, AMBER, title, advice, &f.details));
    }
    rows.push(key_lines(g, info));
    rows.push(label(tr!("crash.help_hint")).caption().margin(Insets::new(4.0, 2.0, 0.0, 2.0)).into());
    let body = scroll().grow(1.0).min_height(0.0).align(Align::Stretch).padding(Insets::new(12.0, 18.0, 18.0, 18.0)).gap(10.0).children(rows);

    let summary = summary(g, info);
    let game_dir = g.m.instance(&info.instance).map(|i| i.game_dir.clone());
    let footer = row()
        .fill_width()
        .padding(Insets::new(10.0, 14.0, 10.0, 14.0))
        .gap(8.0)
        .justify(Justify::End)
        .child(button(tr!("crash.copy")).icon(Icon::Copy).on_click({
            let summary = summary.clone();
            move |g: &mut Gui| g.copy_text(&summary)
        }))
        .child(button(tr!("crash.open_dir")).icon(Icon::Folder).enabled(game_dir.is_some()).on_click(move |g: &mut Gui| {
            if let Some(dir) = &game_dir {
                g.bridge.open_path(dir);
            }
        }))
        .child(button(tr!("crash.export")).accent().icon(Icon::Download).on_click(move |g: &mut Gui| g.bridge.export_crash(&mut g.m, id, summary.clone())));
    frame(header(lead, Vec::new()), body.into(), Some(footer.into()))
}

/// Instance name, exit code and run time.
fn meta_line(g: &Gui, info: &CrashInfo) -> String {
    let mut parts = vec![instance_name(g, &info.instance)];
    if let CrashKind::Game { code, run_secs, .. } = &info.kind {
        if let Some(code) = code {
            parts.push(tr!("crash.exit_code", code = format_exit_code(*code)));
        }
        parts.push(tr!("crash.run_time", time = format::duration(*run_secs)));
    }
    parts.join(" · ")
}

pub(super) fn instance_name(g: &Gui, id: &str) -> String {
    g.m.instance(id).map(|i| i.settings.display_name.clone().unwrap_or_else(|| i.name.clone())).unwrap_or_else(|| id.to_string())
}

/// Title and advice for a cause, looked up in the current language.
pub(super) fn cause_text(f: &Finding) -> (&'static str, &'static str) {
    let key = f.cause.key();
    (cairn_i18n::tr(&format!("crash.cause.{key}.title")), cairn_i18n::tr(&format!("crash.cause.{key}.advice")))
}

/// One cause card: title, advice and extra details such as mod names or missing dependencies.
fn card(ic: Icon, color: cairn_ui::Color, title: &str, advice: &str, details: &[String]) -> El<Gui> {
    let mut col =
        column().grow(1.0).min_width(0.0).gap(4.0).align(Align::Stretch).child(label(title).weight(700).color(fg(0.95))).child(label(advice).secondary());
    if !details.is_empty() {
        let lines = details.iter().map(|d| label(format!("• {d}")).caption().color(fg(0.82)));
        col = col.child(column().fill_width().gap(2.0).align(Align::Stretch).margin(Insets::new(4.0, 0.0, 0.0, 0.0)).children(lines));
    }
    row()
        .fill_width()
        .align(Align::Start)
        .gap(12.0)
        .padding(Insets::symmetric(14.0, 12.0))
        .background(fg(0.05))
        .corner_radius(12.0)
        .child(icon(ic).icon_size(16.0).color(color).margin(Insets::new(2.0, 0.0, 0.0, 0.0)))
        .child(col)
        .into()
}

/// Key log: a title line that expands to the raw log. For a launch failure, the error message.
fn key_lines(g: &Gui, info: &CrashInfo) -> El<Gui> {
    let open = g.ui.crash_lines_open;
    let title = if info.is_launch_failure() { tr!("crash.raw_error") } else { tr!("crash.key_lines") };
    let toggle = list_item()
        .padding(Insets::symmetric(10.0, 7.0))
        .gap(10.0)
        .selected(open)
        .on_click(|g: &mut Gui| g.ui.crash_lines_open = !g.ui.crash_lines_open)
        .child(icon(Icon::Menu).icon_size(15.0))
        .child(label(title).weight(700).color(fg(0.9)).grow(1.0))
        .child(icon(if open { Icon::ChevronUp } else { Icon::ChevronDown }).icon_size(14.0));
    let lines: Vec<El<Gui>> = if info.analysis.key_lines.is_empty() {
        vec![label(tr!("crash.no_key_lines")).caption().into()]
    } else {
        info.analysis.key_lines.iter().map(|l| label(l.as_str()).font_size(12.0).color(fg(0.78)).into()).collect()
    };
    let text =
        column().fill_width().align(Align::Stretch).gap(2.0).padding(Insets::symmetric(12.0, 10.0)).background(fg(0.045)).corner_radius(10.0).children(lines);
    let text = column().fill_width().align(Align::Stretch).padding(Insets::new(4.0, 0.0, 0.0, 0.0)).child(text);
    column().fill_width().align(Align::Stretch).child(toggle).child(collapse(open).no_appear().key(("crash-lines", info.id)).fill_width().child(text)).into()
}

/// Plain text for copying or exporting: the title, instance info, each cause with its advice, and the key log.
pub fn summary(g: &Gui, info: &CrashInfo) -> String {
    let title = if info.is_launch_failure() { tr!("crash.title_launch") } else { tr!("crash.title_game") };
    let mut s = format!("{title}\n{}\n", meta_line(g, info));
    if info.analysis.findings.is_empty() {
        s.push_str(&format!("\n{}\n{}\n", tr!("crash.unknown_title"), tr!("crash.unknown_advice")));
    }
    for (i, f) in info.analysis.findings.iter().enumerate() {
        let (title, advice) = cause_text(f);
        s.push_str(&format!("\n{}. {title}\n{advice}\n", i + 1));
        for d in &f.details {
            for (j, line) in d.lines().enumerate() {
                s.push_str(if j == 0 { " - " } else { "   " });
                s.push_str(line);
                s.push('\n');
            }
        }
    }
    if let CrashKind::Launch { error, .. } = &info.kind
        && info.analysis.key_lines.is_empty()
    {
        s.push_str(&format!("\n{}:\n{error}\n", tr!("crash.raw_error")));
    }
    if !info.analysis.key_lines.is_empty() {
        let head = if info.is_launch_failure() { tr!("crash.raw_error") } else { tr!("crash.key_lines") };
        s.push_str(&format!("\n{head}:\n"));
        for l in &info.analysis.key_lines {
            s.push_str(l);
            s.push('\n');
        }
    }
    s
}

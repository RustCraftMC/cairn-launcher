//! Small widgets that several panels reuse.

use std::rc::Rc;

use cairn_i18n::tr;

use cairn_bridge::Remote;
use cairn_ui::{Align, Button, El, Icon, Insets, Justify, Rect, Segmented, Styled, button, column, label, list_item, row, tab};

use crate::gui::Gui;
use crate::style::{RED, fg};

pub fn action_slot(width: f32, child: Option<El<Gui>>) -> El<Gui> {
    row().width(width).height(32.0).shrink(0.0).align(Align::Center).justify(Justify::End).children(child).into()
}

/// A settings row: title and detail on the left, the control on the right.
pub fn setting_row(title: &str, detail: &str, control: impl Into<El<Gui>>) -> El<Gui> {
    let mut text = column().grow(1.0).min_width(0.0).gap(2.0).child(label(title).weight(700).color(fg(0.92)));
    if !detail.is_empty() {
        text = text.child(label(detail).caption());
    }
    row().fill_width().gap(16.0).padding(Insets::new(6.0, 6.0, 6.0, 8.0)).child(text).child(control).into()
}

/// Segmented control: mutually exclusive options sit on a faint background, like one control.
/// The selected background slides from the old option to the new one when switching.
/// Its right edge lines up with the button next to it.
pub fn seg_track() -> Segmented<Gui> {
    cairn_ui::segmented().track(fg(0.06))
}

pub fn segmented<T: Copy + PartialEq + 'static>(items: &[(T, &str)], current: T, on: impl Fn(&mut Gui, T) + 'static) -> El<Gui> {
    let on = Rc::new(on);
    seg_track()
        .options(items.iter().map(|(value, name)| {
            let (value, on) = (*value, on.clone());
            tab(*name).selected(current == value).on_click(move |g: &mut Gui| on(g, value))
        }))
        .into()
}

/// A setting that can follow the global value. The tags at the top right switch between
/// following and set alone. `toggle(true)` switches to set alone and should write a starting
/// value. `toggle(false)` switches back to following and should clear it. While following,
/// `global` is shown below; otherwise the `editor` control is shown.
pub fn overridable(
    title: &str,
    detail: &str,
    own: bool,
    global: impl Into<String>,
    toggle: impl Fn(&mut Gui, bool) + 'static,
    editor: impl FnOnce() -> El<Gui>,
) -> El<Gui> {
    let mode = segmented(&[(false, tr!("controls.follow_global")), (true, tr!("controls.custom_setting"))], own, toggle);
    let mut head = column().grow(1.0).min_width(0.0).gap(2.0).child(label(title).weight(700).color(fg(0.92)));
    if !detail.is_empty() {
        head = head.child(label(detail).caption());
    }
    let mut col = column().fill_width().gap(8.0).padding(Insets::new(8.0, 6.0, 8.0, 8.0)).child(row().fill_width().gap(16.0).child(head).child(mode));
    col = if own { col.child(editor()) } else { col.child(label(tr!("controls.using_global", value = global.into())).caption()) };
    col.into()
}

/// Lists the Java installs on this machine, one row each: version, vendor, architecture,
/// whether the launcher downloaded it, and the install path.
/// Two Java 17 builds only differ by vendor and path, so the rows are shown, not a dropdown.
pub fn java_list(g: &Gui, chosen: Option<&std::path::Path>, on_pick: impl Fn(&mut Gui, std::path::PathBuf) + 'static) -> Vec<El<Gui>> {
    let on_pick = Rc::new(on_pick);
    match &g.m.javas {
        Remote::Ready(list) => list
            .iter()
            .map(|j| {
                let (path, on_pick) = (j.path.clone(), on_pick.clone());
                let mut info = vec![j.vendor.clone().unwrap_or_else(|| tr!("version.vendor_unknown").into()), j.arch.clone()];
                if j.managed {
                    info.push(tr!("version.managed_download").into());
                }
                list_item()
                    .key(j.path.to_string_lossy().into_owned())
                    .selected(chosen == Some(j.path.as_path()))
                    .on_click(move |g: &mut Gui| on_pick(g, path.clone()))
                    .child(
                        column()
                            .grow(1.0)
                            .min_width(0.0)
                            .gap(2.0)
                            .child(label(format!("Java {}", j.version)).weight(700).color(fg(0.92)))
                            .child(label(format!("{} · {}", info.join(" · "), j.home.display())).caption().truncate()),
                    )
                    .into()
            })
            .collect(),
        Remote::Failed(e) => vec![label(tr!("settings.java_scan_failed", error = e)).caption().color(RED).margin(Insets::symmetric(6.0, 8.0)).into()],
        _ => vec![label(tr!("version.scanning_javas")).caption().margin(Insets::symmetric(6.0, 8.0)).into()],
    }
}

pub fn dropdown_button(label: impl Into<String>, on_open: impl Fn(&mut Gui, Rect) + 'static) -> Button<Gui> {
    button(label).icon_right(Icon::ChevronDown).on_click_rect(on_open)
}

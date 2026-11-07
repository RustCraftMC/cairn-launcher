//! Context menu. While the layout is locked it shows the widget's options and Edit home page.
//! Edit mode adds size presets and Remove, and the blank-area menu adds Tidy and Reset.

use cairn_i18n::tr;
use cairn_ui::{Align, El, Icon, Insets, Styled, checkbox, column, divider, label, list_item, panel, reveal, row, tab};

use super::glyph::{Glyph, glyph};
use super::kinds::{self, Opt, span};
use super::{MenuState, close_menu, committed, enter_edit, exit_edit, remove, reset, resize_to, set_option, tidy, toggle_empty_hint};
use crate::gui::Gui;
use crate::style::{RED, fg};

const WIDTH: f32 = 248.0;
const ROW_H: f32 = 34.0;

fn entry(g: Glyph, text: &str, color: cairn_ui::Color, f: impl Fn(&mut Gui) + 'static) -> El<Gui> {
    list_item()
        .fill_width()
        .padding(Insets::symmetric(10.0, 7.0))
        .gap(10.0)
        .on_click(f)
        .child(glyph(g).size(15.0).color(color.fade(0.85)))
        .child(label(text).font_size(13.5).weight(600).color(color))
        .into()
}

fn choice_row(title: &str, tabs: El<Gui>) -> El<Gui> {
    row()
        .fill_width()
        .padding(Insets::symmetric(10.0, 3.0))
        .gap(8.0)
        .child(label(title).font_size(12.5).color(fg(0.5)))
        .child(row().grow(1.0))
        .child(tabs)
        .into()
}

pub fn view(g: &Gui, st: &MenuState, leaving: bool) -> El<Gui> {
    let editing = g.ui.board.editing;
    let items = committed(&g.m);
    let target = st.target.as_ref().and_then(|id| items.iter().find(|it| &it.id == id)).and_then(|it| kinds::find(&it.kind).map(|k| (it, k)));
    let mut body = column().fill_width().gap(1.0).padding(6.0);
    let mut rows = 0.0f32;

    match target {
        Some((it, kind)) => {
            body = body.child(
                row()
                    .fill_width()
                    .padding(Insets::new(6.0, 10.0, 6.0, 10.0))
                    .gap(8.0)
                    .child(glyph(kind.glyph).size(15.0).color(crate::style::accent()))
                    .child(label(kind.name).weight(700).color(fg(0.95)))
                    .child(row().grow(1.0))
                    .child(label(format!("{}×{}", it.w, it.h)).caption()),
            );
            rows += 1.0;
            if editing {
                let mut sizes = row().gap(2.0);
                for p in kind.presets {
                    let id = it.id.clone();
                    let s = p.span;
                    sizes = sizes.child(tab(p.label()).selected(span(it.w, it.h) == s).on_click(move |g: &mut Gui| {
                        resize_to(g, &id, s);
                        close_menu(g);
                    }));
                }
                body = body.child(choice_row(tr!("board.menu_size"), sizes.into()));
                rows += 1.0;
            }
            for opt in kind.options {
                let id = it.id.clone();
                match opt {
                    Opt::Choice { key, label: title, choices, default } => {
                        let cur = it.options.get(*key).map(String::as_str).unwrap_or(default);
                        let mut tabs = row().gap(2.0);
                        for (value, name) in choices.iter() {
                            let id = id.clone();
                            let opt_c = crate::board::kinds::opt_choice_label(*value);
                            let lbl = if opt_c.is_empty() { *name } else { opt_c };
                            tabs = tabs.child(tab(lbl).selected(cur == *value).on_click(move |g: &mut Gui| set_option(g, &id, key, value)));
                        }
                        let opt_l = crate::board::kinds::opt_label(title);
                        let row_title = if opt_l.is_empty() { *title } else { opt_l };
                        body = body.child(choice_row(row_title, tabs.into()));
                    }
                    Opt::Toggle { key, label: title, default } => {
                        let cur = it.options.get(*key).map_or(*default, |v| v == "true");
                        body = body.child(
                            row()
                                .fill_width()
                                .padding(Insets::symmetric(10.0, 5.0))
                                .gap(8.0)
                                .child({
                                    let opt_l = crate::board::kinds::opt_label(title);
                                    let t_lbl = if opt_l.is_empty() { *title } else { opt_l };
                                    label(t_lbl).font_size(13.0).color(fg(0.85))
                                })
                                .child(row().grow(1.0))
                                .child(checkbox(cur).on_change(move |g: &mut Gui, v: bool| set_option(g, &id, key, if v { "true" } else { "false" }))),
                        );
                    }
                    Opt::Action { label: title, icon, run } => {
                        let run = *run;
                        let opt_l = crate::board::kinds::opt_label(title);
                        let act_title = if opt_l.is_empty() { *title } else { opt_l };
                        body = body.child(entry(Glyph::Icon(*icon), act_title, fg(0.88), move |g: &mut Gui| {
                            run(g);
                            close_menu(g);
                        }));
                    }
                }
                rows += 1.0;
            }
            body = body.child(divider().fill_width().margin(Insets::symmetric(6.0, 4.0)));
            if let Some(id) = it.kind.strip_prefix(kinds::custom::PREFIX) {
                let id = id.to_string();
                body = body.child(entry(Glyph::Icon(Icon::Gear), tr!("extensions.manage_owner"), fg(0.88), move |g: &mut Gui| {
                    crate::views::extensions::open(g, Some(&id))
                }));
                rows += 1.0;
            } else if let Some(rest) = it.kind.strip_prefix(kinds::custom::PLUGIN_PREFIX) {
                let plugin = rest.split_once('/').map_or(rest, |(p, _)| p).to_string();
                body = body.child(entry(Glyph::Icon(Icon::Gear), tr!("extensions.manage_owner"), fg(0.88), move |g: &mut Gui| {
                    crate::views::extensions::open_plugin(g, &plugin)
                }));
                rows += 1.0;
            }
            if editing {
                let id = it.id.clone();
                body = body.child(entry(Glyph::Icon(Icon::Trash), tr!("board.menu_remove"), RED, move |g: &mut Gui| remove(g, &id)));
            } else {
                body = body.child(entry(Glyph::Grid, tr!("board.menu_edit_board"), fg(0.88), enter_edit));
            }
            rows += 1.0;
        }
        None => {
            if editing {
                body = body
                    .child(entry(Glyph::Icon(Icon::Check), tr!("board.menu_finish_edit"), fg(0.88), |g: &mut Gui| {
                        exit_edit(g);
                        close_menu(g);
                    }))
                    .child(divider().fill_width().margin(Insets::symmetric(6.0, 4.0)))
                    .child(entry(Glyph::Icon(Icon::Menu), tr!("board.menu_tidy"), fg(0.88), tidy))
                    .child(entry(Glyph::Icon(Icon::Refresh), tr!("board.menu_reset_default"), fg(0.88), reset));
                rows += 3.0;
            } else {
                body = body.child(entry(Glyph::Grid, tr!("board.menu_edit_board"), fg(0.88), enter_edit)).child(entry(
                    Glyph::Icon(Icon::Plus),
                    tr!("board.menu_gallery"),
                    fg(0.88),
                    enter_edit,
                ));
                rows += 2.0;
                if items.is_empty() {
                    let hidden = g.m.board.as_ref().is_some_and(|l| l.hide_empty_hint);
                    body = body.child(entry(
                        Glyph::Icon(if hidden { Icon::Plus } else { Icon::Minus }),
                        if hidden { tr!("board.menu_show_hint") } else { tr!("board.menu_hide_hint") },
                        fg(0.88),
                        toggle_empty_hint,
                    ));
                    rows += 1.0;
                }
            }
        }
    }

    // The menu opens below and right of the pointer. Near the right or bottom edge of the
    // home page, it flips to the other side.
    let m = g.ui.board.metrics.get();
    let height = rows * ROW_H + 30.0;
    let area = if m.ready() { m.canvas } else { cairn_ui::Rect::new(0.0, 0.0, 1100.0, 700.0) };
    let x = if st.at.x + WIDTH > area.right() { (st.at.x - WIDTH).max(area.x) } else { st.at.x };
    let y = if st.at.y + height > area.bottom() + 40.0 { (st.at.y - height).max(area.y) } else { st.at.y };

    reveal()
        .from_above()
        .offset(6.0)
        .duration(std::time::Duration::from_millis(160))
        .key(("board-menu", st.seq))
        .leaving(leaving)
        .on_left(|g: &mut Gui| g.ui.board.menu_leaving = None)
        .absolute()
        .left(x)
        .top(y)
        .width(WIDTH)
        .child(panel().popover().padding(0.0).gap(0.0).align(Align::Stretch).child(body))
        .into()
}

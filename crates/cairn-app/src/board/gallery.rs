//! Widget gallery panel displayed at the bottom of the screen during board editing.
//!
//! Provides live previews, category filtering, search, and size selection.
//! Widgets can be added via the button, double-clicked, or dragged onto the grid.

use cairn_i18n::tr;
use std::collections::BTreeMap;

use cairn_ui::{Align, El, Icon, Insets, Justify, Styled, button, collapse, column, divider, icon_button, label, panel, reveal, row, scroll, tab, text_input};

use super::fx::preview_box;
use super::glyph::{Glyph, glyph};
use super::kinds::{self, Category, Kind};
use super::{Gesture, Mode, add, on_preview, render};
use crate::gui::Gui;
use crate::style::{accent, fg};

const WIDTH: f32 = 940.0;
/// Total panel height when expanded.
const HEIGHT: f32 = 300.0;
/// Header row height. The remaining height (`HEIGHT - HEADER_HEIGHT`) gives
/// the collapsible list a definite height for scrolling.
const HEADER_HEIGHT: f32 = 58.0;
const CARD_W: f32 = 206.0;
const PREVIEW_W: f32 = 186.0;
const PREVIEW_H: f32 = 116.0;

/// Whether the gallery should fade out during drag gestures:
/// always fades for move and resize, but only fades for incoming widgets
/// once the pointer enters the grid.
fn faded(g: &Gui, gs: &Gesture) -> bool {
    match gs.mode {
        Mode::Incoming { pointer, .. } => g.ui.board.metrics.get().grid.contains(pointer),
        _ => true,
    }
}

pub fn view(g: &Gui) -> El<Gui> {
    let b = &g.ui.board;
    let hide = !b.editing || b.gesture.as_ref().is_some_and(|gs| faded(g, gs));
    let q = b.query.trim().to_lowercase();
    kinds::custom::sync(&g.m);
    let shown: Vec<&'static Kind> = kinds::all()
        .into_iter()
        .filter(|k| b.category.is_none_or(|c| k.category == c))
        .filter(|k| q.is_empty() || k.name().to_lowercase().contains(&q) || k.summary().to_lowercase().contains(&q))
        .collect();

    let mut tabs = row().gap(2.0).child(tab(tr!("board.gallery_all")).selected(b.category.is_none()).on_click(|g: &mut Gui| g.ui.board.category = None));
    for c in Category::ALL {
        tabs = tabs.child(tab(c.label()).selected(b.category == Some(c)).on_click(move |g: &mut Gui| g.ui.board.category = Some(c)));
    }
    let header = row()
        .fill_width()
        .padding(Insets::new(10.0, 10.0, 10.0, 16.0))
        .gap(10.0)
        .child(glyph(Glyph::Grid).size(16.0).color(accent()))
        .child(
            text_input(&b.query)
                .icon(Icon::Search)
                .placeholder(tr!("board.gallery_widgets"))
                .width(190.0)
                .on_change(|g: &mut Gui, s: String| g.ui.board.query = s),
        )
        .child(tabs)
        .child(row().grow(1.0))
        .child(label(tr!("board.gallery_drag_hint")).caption().truncate().min_width(0.0).shrink(1.0))
        .child(icon_button(Icon::Gear).label(tr!("extensions.manage")).on_click(|g: &mut Gui| crate::views::extensions::open(g, None)))
        .child(
            icon_button(if b.collapsed { Icon::ChevronUp } else { Icon::ChevronDown })
                .label(if b.collapsed { tr!("board.gallery_expand") } else { tr!("board.gallery_collapse") })
                .on_click(|g: &mut Gui| {
                    let b = &mut g.ui.board;
                    b.collapsed = !b.collapsed;
                    // Keep the body mounted during collapse animation and mount it before
                    // expand animation starts.
                    b.body_gone = false;
                }),
        );

    // The collapsible body below the header animates height and fades content.
    // Skip the initial appear animation because the panel has its own entrance.
    // Stop building the body once collapsed.
    let open = !b.collapsed;
    let mut drawer = collapse(open).no_appear().key("board-gallery-body").fill_width().on_collapsed(|g: &mut Gui| g.ui.board.body_gone = true);
    if open || !b.body_gone {
        let body: El<Gui> = if shown.is_empty() && b.category == Some(Category::Custom) && q.is_empty() {
            column()
                .fill_width()
                .grow(1.0)
                .align(Align::Center)
                .justify(Justify::Center)
                .gap(10.0)
                .child(label(tr!("board.gallery_no_custom")).weight(700).color(fg(0.85)))
                .child(label(tr!("board.gallery_custom_hint")).caption())
                .child(button(tr!("board.gallery_reload")).icon(Icon::Refresh).on_click(|g: &mut Gui| g.bridge.reload_panels(&mut g.m)))
                .into()
        } else if shown.is_empty() {
            column().fill_width().grow(1.0).align(Align::Center).justify(Justify::Center).child(label(tr!("board.gallery_no_match")).caption()).into()
        } else {
            scroll().grow(1.0).padding(Insets::new(6.0, 14.0, 14.0, 14.0)).child(row().wrap(true).gap(12.0).children(shown.iter().map(|k| card(g, k)))).into()
        };
        drawer = drawer.child(column().fill_width().height(HEIGHT - HEADER_HEIGHT).align(Align::Stretch).child(divider().fill_width().shrink(0.0)).child(body));
    }
    let p = panel().popover().padding(0.0).gap(0.0).align(Align::Stretch).fill_width().child(header.min_height(HEADER_HEIGHT).shrink(0.0)).child(drawer);

    reveal()
        .key("board-gallery")
        .leaving(hide)
        .offset(28.0)
        .on_left(|g: &mut Gui| {
            if !g.ui.board.editing {
                g.ui.board.gallery = false;
            }
        })
        .max_width(WIDTH)
        .fill_width()
        .child(p)
        .into()
}

fn card(g: &Gui, kind: &'static Kind) -> El<Gui> {
    let b = &g.ui.board;
    let sel = b.sizes.get(kind.id).copied().unwrap_or(kind.default).min(kind.presets.len() - 1);
    let s = kind.presets[sel].span;
    let size = b.metrics.get().tile_size(s);
    let k = (PREVIEW_W / size.width).min(PREVIEW_H / size.height).min(0.8);
    let content = render(g, kind, s, &BTreeMap::new(), true).absolute().left(0.0).top(0.0).width(size.width).height(size.height);
    let preview = preview_box::<Gui>(k)
        .width(size.width * k)
        .height(size.height * k)
        .on_drag(move |g: &mut Gui, ev| on_preview(g, kind, s, k, ev))
        .child(content)
        .key(("preview", kind.id));

    let mut sizes = row().fill_width().gap(2.0);
    for (i, p) in kind.presets.iter().enumerate() {
        sizes = sizes.child(tab(p.label()).selected(i == sel).on_click(move |g: &mut Gui| {
            g.ui.board.sizes.insert(kind.id, i);
        }));
    }
    sizes = sizes.child(row().grow(1.0)).child(label(format!("{}×{}", s.w, s.h)).font_size(11.5).color(fg(0.4)));

    column()
        .width(CARD_W)
        .padding(10.0)
        .gap(8.0)
        .background(fg(0.035))
        .corner_radius(16.0)
        .key(("card", kind.id))
        .child(row().fill_width().height(PREVIEW_H).justify(Justify::Center).child(preview))
        .child(
            row()
                .fill_width()
                .gap(8.0)
                .child(
                    column()
                        .grow(1.0)
                        .min_width(0.0)
                        .gap(1.0)
                        .child(label(kind.name()).weight(700).color(fg(0.94)).truncate())
                        .child(label(kind.summary()).font_size(11.5).color(fg(0.5)).truncate()),
                )
                .child(button(tr!("board.gallery_add")).icon(Icon::Plus).on_click(move |g: &mut Gui| {
                    add(g, kind, s);
                })),
        )
        .child(sizes)
        .into()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use cairn_bridge::{Appearance, Bridge, Remote, Sink, Status};
    use cairn_i18n::Language;
    use cairn_ui::testing::Headless;

    use super::*;

    fn settle(app: &mut Headless<Gui>) {
        app.render();
        for _ in 0..40 {
            app.run_animations(600);
            app.advance(Duration::from_millis(100));
            app.render();
            if !app.animating() && !app.redraw_requested() {
                break;
            }
        }
    }

    /// Initializes a headless window with the widget gallery opened in edit mode.
    fn editing(rt: &tokio::runtime::Runtime) -> Headless<Gui> {
        let sink: Sink = Arc::new(|_| true);
        let mut g = Gui::new(Bridge::start(rt.handle().clone(), sink));
        g.m.status = Status::Ready;
        g.m.config.setup_completed = true;
        g.m.config.eula_accepted = true;
        g.m.instances = Remote::Ready(Vec::new());
        crate::board::enter_edit(&mut g);
        let theme = crate::style::theme(&Appearance::default(), None, false);
        let text = cairn_ui::TextSystem::new(&cairn_ui::testing::system_font_setup());
        let mut app = Headless::with_theme(g, Gui::view, 1120.0, 700.0, 1.0, theme, text);
        settle(&mut app);
        app
    }

    /// Runs the animation to completion, recording the header y-position at each frame.
    fn play(app: &mut Headless<Gui>, top: impl Fn(&Headless<Gui>) -> f32) -> Vec<f32> {
        app.render();
        let mut ys = Vec::new();
        for _ in 0..60 {
            if !app.animating() {
                break;
            }
            app.run_animations(1);
            ys.push(top(app));
        }
        app.advance(Duration::from_millis(50));
        app.render();
        ys
    }

    #[test]
    fn collapsing_and_expanding_the_gallery_is_animated() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        cairn_i18n::with_language(Language::ZhCn, || {
            let mut app = editing(&rt);
            let top = |a: &Headless<Gui>| a.find(tr!("board.gallery_drag_hint")).expect("标题行").bounds.y;
            let open = top(&app);
            assert!(app.find(tr!("board.gallery_add")).is_some(), "展开时有小组件卡片");

            app.update(|g| {
                g.ui.board.collapsed = true;
                g.ui.board.body_gone = false;
            });
            let ys = play(&mut app, top);
            let closed = top(&app);
            assert!(closed > open + 150.0, "收起后标题行只下移了 {}", closed - open);
            assert!(ys.windows(2).all(|w| w[1] >= w[0] - 0.01), "收起时标题行应该一路往下:{ys:?}");
            assert!(ys.iter().any(|y| *y > open + 5.0 && *y < closed - 5.0), "收起没有中间状态:{ys:?}");
            assert!(app.state().ui.board.body_gone && app.find(tr!("board.gallery_add")).is_none(), "收起播完里面不再构建");

            app.update(|g| {
                g.ui.board.collapsed = false;
                g.ui.board.body_gone = false;
            });
            let ys = play(&mut app, top);
            assert!(ys.windows(2).all(|w| w[1] <= w[0] + 0.01), "展开时标题行应该一路往上:{ys:?}");
            assert!(ys.iter().any(|y| *y < closed - 5.0 && *y > open + 5.0), "展开没有中间状态:{ys:?}");
            assert!((top(&app) - open).abs() < 1.0, "展开后回到原来的位置");
            assert!(app.find(tr!("board.gallery_add")).is_some());
        });
    }
}

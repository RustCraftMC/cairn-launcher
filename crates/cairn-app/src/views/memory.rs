//! Memory allocation editor: Auto or Custom, with a bar for used, game and machine memory.
//! Custom adds a slider. Shared by the global and instance settings.

use std::rc::Rc;

use cairn_bridge::MemoryPolicy;
use cairn_i18n::tr;
use cairn_ui::{Align, El, Insets, Styled, column, label, row, slider};

use super::controls::segmented;
use crate::gui::Gui;
use crate::style::{RED, accent, fg};
use crate::widgets::memory_bar;

const GIB: f64 = 1024.0;

fn gb(mib: f64) -> String {
    format!("{:.1} GB", mib / GIB)
}

/// `policy` is the policy being edited and `on_change` receives the changed one.
/// `instance` is the instance the Auto estimate refers to.
pub fn block(g: &Gui, policy: MemoryPolicy, instance: Option<&str>, on_change: impl Fn(&mut Gui, MemoryPolicy) + 'static) -> El<Gui> {
    let on_change = Rc::new(on_change);
    let total = g.m.memory.map(|m| m.total as f64 / (1024.0 * 1024.0)).unwrap_or(8192.0);
    let free = g.m.memory.map(|m| m.available as f64 / (1024.0 * 1024.0)).unwrap_or(total / 2.0);
    let used = (total - free).max(0.0);
    // Only an estimate made for this exact instance is trusted. Otherwise estimate as vanilla.
    let auto = g.m.memory_hint.as_ref().filter(|h| h.instance.as_deref() == instance).map(|h| h.auto_mib as f64).unwrap_or(4096.0);
    let game = match policy {
        MemoryPolicy::Auto => auto,
        MemoryPolicy::Fixed { mib } => mib as f64,
    };
    let max = ((total / 256.0).floor() * 256.0).max(1024.0) as f32;

    let is_auto = matches!(policy, MemoryPolicy::Auto);
    let oc = on_change.clone();
    let mode = segmented(&[(true, tr!("memory.auto_calc")), (false, tr!("memory.manual"))], is_auto, move |g: &mut Gui, auto| {
        let p = if auto { MemoryPolicy::Auto } else { MemoryPolicy::Fixed { mib: (auto_default(g)).min(max as u32) } };
        oc(g, p);
    });

    let legend = |color: cairn_ui::Color, text: String| {
        row().gap(6.0).child(cairn_ui::icon(cairn_ui::Icon::Dot).icon_size(10.0).color(color)).child(label(text).caption())
    };
    let mut col =
        column().fill_width().gap(10.0).align(Align::Stretch).padding(Insets::new(2.0, 0.0, 4.0, 0.0)).child(mode).child(memory_bar(total, used, game)).child(
            row()
                .fill_width()
                .gap(16.0)
                .child(legend(fg(0.3), tr!("memory.other_used", size = gb(used))))
                .child(legend(accent(), tr!("memory.game_alloc", size = gb(game))))
                .child(row().grow(1.0))
                .child(label(tr!("memory.total_space", size = gb(total))).caption()),
        );
    if let MemoryPolicy::Fixed { mib } = policy {
        let oc = on_change.clone();
        col = col.child(
            row()
                .fill_width()
                .gap(14.0)
                .child(
                    slider(mib as f32)
                        .range(512.0, max)
                        .step(256.0)
                        .grow(1.0)
                        .on_change(move |g: &mut Gui, v: f32| oc(g, MemoryPolicy::Fixed { mib: v.round() as u32 })),
                )
                .child(label(gb(mib as f64)).weight(700).color(fg(0.9)).width(72.0)),
        );
        if game > free {
            col = col.child(label(tr!("memory.free_memory_low", size = gb(free))).caption().color(RED));
        }
    } else {
        col = col.child(label(tr!("memory.memory_auto_desc")).caption());
    }
    col.into()
}

/// Starting value when switching from Auto to Custom: the auto estimate, or 4 GB without one.
fn auto_default(g: &Gui) -> u32 {
    g.m.memory_hint.as_ref().map(|h| h.auto_mib).unwrap_or(4096).max(1024)
}

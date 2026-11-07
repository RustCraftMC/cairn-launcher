//! Desktop UI of CairnLauncher. Business logic and async work live in `cairn-bridge`.
//! This crate holds the app state and root view (`gui`), the panels (`views/`) and styling.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// Wraps a custom widget into an element and implements `cairn_ui::Styled` for it.
/// The style, key and (for containers) children move onto the element.
/// The widget needs `style: Style` and `key: Option<u64>` fields. Containers also need a `children` field.
macro_rules! into_el {
    (container $ty:ident) => {
        impl<S: 'static> From<$ty<S>> for cairn_ui::El<S>
        where
            $ty<S>: cairn_ui::Widget<S>,
        {
            fn from(mut w: $ty<S>) -> Self {
                let style = std::mem::take(&mut w.style);
                let key = w.key.take();
                let children = std::mem::take(&mut w.children);
                let mut el = cairn_ui::El::new(w).style(style).children(children);
                if let Some(k) = key {
                    el = cairn_ui::Styled::key(el, k);
                }
                el
            }
        }

        impl<S: 'static> cairn_ui::Styled for $ty<S> {
            fn style_mut(&mut self) -> &mut cairn_ui::Style {
                &mut self.style
            }
            fn key_mut(&mut self) -> &mut Option<u64> {
                &mut self.key
            }
        }
    };
    ($ty:ident $(<$s:ident>)?) => {
        impl<S: 'static> From<$ty $(<$s>)?> for cairn_ui::El<S>
        where
            $ty $(<$s>)?: cairn_ui::Widget<S>,
        {
            fn from(mut w: $ty $(<$s>)?) -> Self {
                let style = std::mem::take(&mut w.style);
                let key = w.key.take();
                let mut el = cairn_ui::El::new(w).style(style);
                if let Some(k) = key {
                    el = cairn_ui::Styled::key(el, k);
                }
                el
            }
        }

        impl $(<$s: 'static>)? cairn_ui::Styled for $ty $(<$s>)? {
            fn style_mut(&mut self) -> &mut cairn_ui::Style {
                &mut self.style
            }
            fn key_mut(&mut self) -> &mut Option<u64> {
                &mut self.key
            }
        }
    };
}

mod board;
mod gui;
mod scene;
#[cfg(test)]
mod screenshots;
mod style;
mod views;
mod widgets;

use std::cell::OnceCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use cairn_bridge::{Appearance, Bridge, Model, Sink};
use cairn_ui::App;

use crate::gui::Gui;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The launcher starts itself with this flag to run script plugins in a separate process.
    if std::env::args().any(|arg| arg == cairn_bridge::PLUGIN_HOST_FLAG) {
        cairn_bridge::run_plugin_host();
        return Ok(());
    }
    // Recovery: start without running any script plugin, for example after one breaks launching.
    if std::env::args().any(|arg| arg == "--no-plugins") {
        cairn_bridge::disable_plugins();
    }
    cairn_bridge::cleanup_old_executable();
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().thread_name("cairn-rt").build()?;
    let handle = rt.handle().clone();
    // Read the appearance and wallpaper while the window is created, so the first frame has them.
    let startup = std::thread::spawn(cairn_bridge::load_startup_appearance);
    // The state is dropped when the window closes, so keep a second handle to flush unsaved settings.
    let bridge_slot: Rc<OnceCell<Bridge>> = Rc::new(OnceCell::new());
    let slot = bridge_slot.clone();

    let mut app = App::new(Gui::view)
        .title("CairnLauncher")
        .size(1120.0, 700.0)
        .min_size(960.0, 600.0)
        .max_size(1440.0, 900.0)
        .maximizable(false)
        .decorations(false)
        .theme(style::theme(&Appearance::default(), None, false))
        .theme_from(Gui::theme_if_changed)
        .on_file_drop(Gui::on_file_drop);

    if let Ok((w, h, rgba)) = cairn_bridge::decode_image(include_bytes!("../../../assets/logo.png")) {
        app = app.icon(rgba, w, h);
    }

    let result = app.run_with(move |proxy| {
        let sink: Sink = Arc::new(move |f: Box<dyn FnOnce(&mut Model) + Send>| {
            proxy.update(move |g: &mut Gui| {
                f(&mut g.m);
                g.after_update();
            })
        });
        let bridge = Bridge::start(handle, sink);
        let _ = slot.set(bridge.clone());
        let mut gui = Gui::new(bridge);
        if let Ok(startup) = startup.join() {
            startup.apply(&mut gui.m);
        }
        gui
    });

    if let Some(bridge) = bridge_slot.get() {
        bridge.shutdown(&rt);
    }
    rt.shutdown_timeout(Duration::from_secs(1));
    result.map_err(Into::into)
}

//! Accounts: select and delete accounts, add offline accounts, and add external auth accounts.
//! External auth covers LittleSkin and Authlib-Injector. Microsoft sign-in uses a device code.

use cairn_bridge::{AccountInfo, AccountKind, DeviceLoginStatus, MsLogin, Remote, YggLogin};
use cairn_i18n::tr;
use cairn_ui::{
    Affine, Align, Change, Easing, El, Icon, Insets, Justify, LayoutCx, PaintCx, Painter, Point, Style, Styled, Transition, Widget, button, click_area, column,
    divider, icon, icon_button, label, list_item, panel, reveal, row, scroll, spinner, tag, text_input,
};

use super::controls::{action_slot, segmented};
use super::{empty_state, frame, header, loading_row, removable, swap};
use crate::gui::{AddAccountTab, Confirm, Gui, Removal};
use crate::style::{RED, accent, fg};
use crate::widgets::{initial, tile};

impl Gui {
    fn add_offline(&mut self) {
        let name = self.ui.offline_name.trim().to_string();
        if name.is_empty() {
            return;
        }
        self.bridge.add_offline_account(&mut self.m, &name);
        self.ui.offline_name.clear();
        self.close_add_account();
    }

    fn add_yggdrasil(&mut self) {
        let server = if self.ui.ygg_custom {
            let s = self.ui.ygg_server.trim();
            if s.is_empty() { "https://littleskin.cn/api/yggdrasil" } else { s }
        } else {
            "https://littleskin.cn/api/yggdrasil"
        };
        let user = self.ui.ygg_username.trim().to_string();
        let pass = self.ui.ygg_password.clone();
        if user.is_empty() || pass.is_empty() {
            return;
        }
        self.bridge.add_yggdrasil_account(&mut self.m, server, &user, &pass);
        self.ui.ygg_password.clear();
    }
}

pub fn view(g: &Gui) -> El<Gui> {
    let mut rows: Vec<El<Gui>> = Vec::new();
    match &g.m.accounts {
        Remote::Ready(list) => {
            if list.is_empty() {
                rows.push(
                    column()
                        .fill_width()
                        .grow(1.0)
                        .align(Align::Center)
                        .justify(Justify::Center)
                        .padding(Insets::symmetric(48.0, 16.0))
                        .gap(14.0)
                        .child(empty_state(
                            Icon::User,
                            tr!("accounts.empty"),
                            if g.m.can_add_offline_accounts() { tr!("accounts.empty_desc") } else { tr!("accounts.empty_desc_microsoft") },
                        ))
                        .child(button(tr!("accounts.add_title")).icon(Icon::Plus).accent().on_click(|g: &mut Gui| g.open_add_account()))
                        .into(),
                );
            } else {
                rows.extend(list.iter().map(|a| removable(g, Removal::Account(a.id.clone()), account_row(g, a))));
            }
        }
        Remote::Failed(e) => rows.push(label(tr!("accounts.fetch_failed", error = e)).caption().color(RED).into()),
        _ => rows.push(loading_row(tr!("accounts.reading"))),
    }

    let accounts_list = scroll().grow(1.0).align(Align::Stretch).padding(Insets::new(10.0, 14.0, 18.0, 14.0)).gap(4.0).children(rows);

    let mut body = column().grow(1.0).min_height(0.0).fill_width().align(Align::Stretch).child(accounts_list);

    if g.ui.add_account_open {
        body = body.child(click_area().absolute().top(0.0).right(0.0).bottom(0.0).left(0.0).on_click(|g: &mut Gui| g.close_add_account()));
    }
    if g.ui.add_account_open || g.ui.add_account_leaving {
        body = body.child(
            reveal()
                .from_above()
                .absolute()
                .top(8.0)
                .right(16.0)
                .width(520.0)
                .key("add_account_popover")
                .leaving(!g.ui.add_account_open)
                .on_left(|g: &mut Gui| g.ui.add_account_leaving = false)
                .child(add_account_popover(g)),
        );
    }

    let actions = vec![button(tr!("accounts.add_title")).icon(Icon::Plus).accent().on_click(|g: &mut Gui| g.toggle_add_account()).into()];

    frame(header(label(tr!("accounts.title")).title(), actions), body.into(), None)
}

fn add_account_popover(g: &Gui) -> El<Gui> {
    let head = row()
        .fill_width()
        .padding(Insets::new(14.0, 14.0, 10.0, 18.0))
        .align(Align::Center)
        .child(
            row()
                .gap(8.0)
                .align(Align::Center)
                .grow(1.0)
                .child(icon(Icon::User).icon_size(16.0).color(accent()))
                .child(label(tr!("accounts.add_title")).weight(700).font_size(15.0).color(fg(0.95))),
        )
        .child(icon_button(Icon::Close).label(tr!("common.close")).on_click(|g: &mut Gui| g.close_add_account()));

    // Microsoft comes first, and it is the tab selected when the menu opens.
    let tabs = segmented(
        &[
            (AddAccountTab::Microsoft, tr!("accounts.microsoft")),
            (AddAccountTab::Offline, tr!("accounts.offline")),
            (AddAccountTab::Yggdrasil, tr!("accounts.yggdrasil")),
        ],
        g.ui.add_account_tab,
        |g: &mut Gui, t| g.ui.add_account_tab = t,
    );

    let form_content = match g.ui.add_account_tab {
        AddAccountTab::Microsoft => add_microsoft_view(g),
        // Outside mainland China with no Microsoft account, offline and external auth accounts are locked.
        _ if !g.m.can_add_offline_accounts() => locked_view(),
        AddAccountTab::Offline => add_offline_view(g),
        AddAccountTab::Yggdrasil => add_yggdrasil_view(g),
    };

    panel()
        .popover()
        .padding(0.0)
        .gap(0.0)
        .align(Align::Stretch)
        .child(head)
        .child(divider().fill_width())
        .child(column().padding(Insets::new(12.0, 16.0, 16.0, 16.0)).gap(12.0).child(tabs).child(form_content))
        .into()
}

/// Shown in place of the form. The rule lives in `Model::can_add_offline_accounts`.
fn locked_view() -> El<Gui> {
    column()
        .fill_width()
        .gap(10.0)
        .child(
            row()
                .gap(8.0)
                .align(Align::Center)
                .child(icon(Icon::Info).icon_size(16.0).color(accent()))
                .child(label(tr!("accounts.locked_title")).weight(700).color(fg(0.95))),
        )
        .child(label(tr!("accounts.locked_desc")).caption().color(fg(0.65)))
        .child(button(tr!("accounts.locked_action")).icon(Icon::ExternalLink).accent().on_click(|g: &mut Gui| g.ui.add_account_tab = AddAccountTab::Microsoft))
        .into()
}

fn add_offline_view(g: &Gui) -> El<Gui> {
    column()
        .gap(10.0)
        .child(label(tr!("accounts.empty_desc")).caption().color(fg(0.65)))
        .child(
            row()
                .fill_width()
                .gap(8.0)
                .child(
                    text_input(&g.ui.offline_name)
                        .placeholder(tr!("accounts.offline_name_placeholder"))
                        .grow(1.0)
                        .on_change(|g: &mut Gui, s: String| g.ui.offline_name = s)
                        .on_submit(|g: &mut Gui, _| g.add_offline()),
                )
                .child(
                    button(tr!("common.add")).icon(Icon::Plus).accent().enabled(!g.ui.offline_name.trim().is_empty()).on_click(|g: &mut Gui| g.add_offline()),
                ),
        )
        .child(label(tr!("accounts.offline_feature_hint")).caption().color(fg(0.45)))
        .into()
}

fn add_yggdrasil_view(g: &Gui) -> El<Gui> {
    let mode_tabs =
        segmented(&[(false, tr!("accounts.ygg_littleskin_opt")), (true, tr!("accounts.ygg_custom_opt"))], g.ui.ygg_custom, |g: &mut Gui, custom| {
            g.ui.ygg_custom = custom
        });
    let mut col = column().gap(10.0).child(mode_tabs);

    if g.ui.ygg_custom {
        col = col
            .child(text_input(&g.ui.ygg_server).placeholder(tr!("accounts.ygg_server")).fill_width().on_change(|g: &mut Gui, s: String| g.ui.ygg_server = s));
    }

    let can_submit = !g.ui.ygg_username.trim().is_empty() && !g.ui.ygg_password.is_empty();
    let is_logging_in = matches!(g.m.ygg_login, YggLogin::LoggingIn);

    col = col
        .child(
            text_input(&g.ui.ygg_username)
                .placeholder(tr!("accounts.ygg_username"))
                .fill_width()
                .on_change(|g: &mut Gui, s: String| g.ui.ygg_username = s)
                .on_submit(|g: &mut Gui, _| g.add_yggdrasil()),
        )
        .child({
            let mut password = text_input(&g.ui.ygg_password)
                .placeholder(tr!("accounts.ygg_password"))
                .grow(1.0)
                .on_change(|g: &mut Gui, s: String| g.ui.ygg_password = s)
                .on_submit(|g: &mut Gui, _| g.add_yggdrasil());
            if !g.ui.ygg_password_shown {
                password = password.secret();
            }
            row().fill_width().gap(6.0).child(password).child(
                button(if g.ui.ygg_password_shown { tr!("accounts.hide_password") } else { tr!("accounts.show_password") })
                    .width(88.0)
                    .on_click(|g: &mut Gui| g.ui.ygg_password_shown = !g.ui.ygg_password_shown),
            )
        })
        .child(row().fill_width().justify(Justify::End).child(
            button(tr!("accounts.ygg_login_button")).icon(Icon::Plus).accent().enabled(can_submit && !is_logging_in).on_click(|g: &mut Gui| g.add_yggdrasil()),
        ));

    let mut status = column().fill_width().min_height(18.0);
    if is_logging_in {
        status = status
            .child(row().height(18.0).gap(8.0).padding(Insets::symmetric(4.0, 0.0)).child(spinner()).child(label(tr!("accounts.ygg_logging_in")).caption()));
    } else if let YggLogin::Failed(e) = &g.m.ygg_login {
        status = status.child(label(tr!("auth.ms.failed", error = e)).caption().color(RED));
    }
    col = col.child(status);

    col = col.child(label(tr!("accounts.ygg_authlib_hint")).caption().color(fg(0.45)));

    col.into()
}

fn add_microsoft_view(g: &Gui) -> El<Gui> {
    if !g.m.ms_available {
        return column()
            .gap(8.0)
            .child(button(tr!("auth.ms.title")).icon(Icon::ExternalLink).enabled(false))
            .child(label(tr!("auth.ms.no_client_id")).caption().color(fg(0.55)))
            .into();
    }
    let start = || button(tr!("auth.ms.title")).icon(Icon::ExternalLink).accent().on_click(|g: &mut Gui| g.bridge.begin_ms_login(&mut g.m));
    match &g.m.ms_login {
        MsLogin::Idle => column().gap(8.0).child(start()).child(label(tr!("auth.ms.safe_tip")).caption().color(fg(0.55))).into(),
        MsLogin::Requesting => loading_row(tr!("auth.ms.requesting")),
        MsLogin::Succeeded { account_id, name } => microsoft_success_view(account_id, name),
        MsLogin::Active { user_code, verification_uri, deadline, status, code_copied } => {
            let uri = verification_uri.clone();
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            let secs = remaining.as_secs();
            let time_str = format!("{:02}:{:02}", secs / 60, secs % 60);

            let status_text = match status {
                DeviceLoginStatus::Idle | DeviceLoginStatus::RequestingDeviceCode => tr!("auth.ms.requesting").to_string(),
                DeviceLoginStatus::WaitingForUser | DeviceLoginStatus::AuthorizationPending => {
                    tr!("auth.ms.waiting_remaining", time = time_str)
                }
                DeviceLoginStatus::Polling => {
                    tr!("auth.ms.polling", time = time_str)
                }
                DeviceLoginStatus::SlowDown => {
                    tr!("auth.ms.slow_down", time = time_str)
                }
                DeviceLoginStatus::ExchangingToken => tr!("auth.ms.exchanging").to_string(),
                DeviceLoginStatus::Authenticated => tr!("auth.ms.authenticated").to_string(),
                DeviceLoginStatus::Expired => tr!("auth.ms.expired").to_string(),
                DeviceLoginStatus::Declined => tr!("auth.ms.declined").to_string(),
                DeviceLoginStatus::Cancelled => tr!("auth.ms.cancelled").to_string(),
                DeviceLoginStatus::NetworkError(e) => {
                    tr!("auth.ms.network_error", error = e, time = time_str)
                }
                DeviceLoginStatus::ServerError(e) => {
                    tr!("auth.ms.server_error", error = e)
                }
                DeviceLoginStatus::Failed(e) => {
                    tr!("auth.ms.failed", error = e)
                }
            };

            let code_card = panel()
                .padding(Insets::symmetric(14.0, 18.0))
                .gap(6.0)
                .align(Align::Center)
                .child(label(tr!("auth.ms.code_label")).caption().color(fg(0.55)))
                .child(
                    row().gap(12.0).align(Align::Center).child(label(user_code).font_size(24.0).weight(700).color(accent())).child(
                        button(if *code_copied { tr!("auth.ms.code_copied") } else { tr!("auth.ms.copy_code") })
                            .width(156.0)
                            .icon(if *code_copied { Icon::Check } else { Icon::Copy })
                            .on_click(|g: &mut Gui| g.bridge.copy_ms_login_code(&mut g.m)),
                    ),
                );

            let uri_for_open = uri.clone();
            column()
                .gap(12.0)
                .child(label(tr!("auth.ms.title")).font_size(15.0).weight(700).color(fg(0.95)))
                .child(label(if *code_copied { tr!("auth.ms.copied_desc") } else { tr!("auth.ms.desc") }).caption().min_height(36.0).color(fg(0.7)))
                .child(row().gap(8.0).align(Align::Center).child(label(tr!("auth.ms.url_label")).caption().color(fg(0.55))).child(label(&uri).color(accent())))
                .child(code_card)
                .child(
                    row()
                        .gap(10.0)
                        .align(Align::Center)
                        .child(button(tr!("auth.ms.open_page")).icon(Icon::ExternalLink).accent().on_click(move |g: &mut Gui| g.bridge.open_url(&uri_for_open)))
                        .child(button(tr!("auth.ms.cancel")).on_click(|g: &mut Gui| g.bridge.cancel_ms_login(&mut g.m))),
                )
                .child(
                    row()
                        .gap(8.0)
                        .align(Align::Center)
                        .padding(Insets::symmetric(4.0, 0.0))
                        .child(spinner())
                        .child(label(status_text).caption().color(fg(0.75))),
                )
                .child(label(tr!("auth.ms.bottom_tip")).caption().color(fg(0.5)))
                .into()
        }
        MsLogin::Declined => column().gap(8.0).child(label(tr!("auth.ms.declined")).caption().color(RED)).child(start()).into(),
        MsLogin::Expired => column().gap(8.0).child(label(tr!("auth.ms.expired")).caption().color(RED)).child(start()).into(),
        MsLogin::NetworkError(e) => column().gap(8.0).child(label(tr!("auth.ms.network_failed", error = e)).caption().color(RED)).child(start()).into(),
        MsLogin::ServerError(e) => column().gap(8.0).child(label(tr!("auth.ms.server_error", error = e)).caption().color(RED)).child(start()).into(),
        MsLogin::Failed(e) => column().gap(8.0).child(label(tr!("auth.ms.failed", error = e)).caption().color(RED)).child(start()).into(),
    }
}

fn microsoft_success_view(account_id: &str, name: &str) -> El<Gui> {
    column()
        .key(("microsoft_success", account_id.to_string()))
        .fill_width()
        .min_height(264.0)
        .padding(Insets::new(20.0, 8.0, 8.0, 8.0))
        .gap(18.0)
        .align(Align::Center)
        .justify(Justify::Center)
        .child(LoginSuccessMark { style: Style::default(), key: None, progress: Transition::default() }.width(72.0).height(72.0))
        .child(
            reveal().key("microsoft_success_text").offset(6.0).child(
                column()
                    .align(Align::Center)
                    .gap(6.0)
                    .child(label(tr!("auth.ms.success_title")).font_size(21.0).weight(700).color(fg(0.95)))
                    .child(label(tr!("auth.ms.success_account", name = name)).font_size(15.0).color(fg(0.8)).max_width(336.0).truncate())
                    .child(label(tr!("auth.ms.success_desc")).caption().color(fg(0.55))),
            ),
        )
        .child(button(tr!("auth.ms.done")).accent().min_width(128.0).on_click(|g: &mut Gui| g.close_add_account()))
        .into()
}

struct LoginSuccessMark {
    style: Style,
    key: Option<u64>,
    progress: Transition,
}

into_el!(LoginSuccessMark);

impl Widget<Gui> for LoginSuccessMark {
    fn update(&mut self, _cx: &mut LayoutCx, _new: Self) -> Change {
        Change::NONE
    }

    fn animate(&mut self, cx: &mut PaintCx) {
        let now = cx.now();
        if cx.in_layer_cache() {
            self.progress.set(1.0);
        }
        self.progress.animate_to(1.0, now, cx.theme().motion.enter.mul_f32(1.5), Easing::SMOOTH);
        if self.progress.is_animating(now) {
            cx.request_frame();
        }
    }

    fn transform(&self, cx: &PaintCx) -> Affine {
        Affine::scale_about(cx.bounds().center(), 0.94 + 0.06 * self.progress.value(cx.now()))
    }

    fn paint(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        let b = cx.bounds().inflate(-3.0, -3.0);
        let t = self.progress.value(cx.now());
        let color = accent();
        p.fill_rounded_rect(b, b.width / 2.0, color.with_opacity(0.10 * t));
        p.stroke_rect(b, b.width / 2.0, 1.0, color.with_opacity(0.28 * t));
        let point = |x, y| Point::new(b.x + b.width * x, b.y + b.height * y);
        let a = point(0.29, 0.51);
        let b = point(0.44, 0.66);
        let c = point(0.73, 0.35);
        let line = |from: Point, to: Point, k: f32, p: &mut dyn Painter| {
            if k > 0.0 {
                p.draw_line(from, Point::new(from.x + (to.x - from.x) * k, from.y + (to.y - from.y) * k), 3.2, color);
            }
        };
        line(a, b, (t * 3.0).clamp(0.0, 1.0), p);
        line(b, c, ((t - 1.0 / 3.0) * 1.5).clamp(0.0, 1.0), p);
    }
}

fn account_row(g: &Gui, a: &AccountInfo) -> El<Gui> {
    let id = a.id.clone();
    let kind = match a.kind {
        AccountKind::Offline => tr!("accounts.offline").to_string(),
        AccountKind::Microsoft if a.needs_refresh => tr!("accounts.needs_refresh").to_string(),
        AccountKind::Microsoft => tr!("accounts.microsoft").to_string(),
        AccountKind::Yggdrasil => {
            if let Some(server) = &a.yggdrasil_server {
                if server.contains("littleskin") {
                    tr!("accounts.ygg_littleskin_account").to_string()
                } else {
                    tr!("accounts.ygg_custom_account", server = server)
                }
            } else {
                tr!("accounts.ygg_generic_account").to_string()
            }
        }
    };
    let mut av = tile(&a.name, initial(&a.name), 38.0).round();
    if let Some(img) = g.icon(&format!("avatar:{}", a.id)) {
        av = av.image(img);
    }
    let mut r = list_item()
        .key(a.id.clone())
        .selected(a.selected)
        .on_click({
            let id = id.clone();
            move |g: &mut Gui| g.bridge.select_account(&mut g.m, &id)
        })
        .child(av)
        .child(column().grow(1.0).min_width(0.0).gap(2.0).child(label(&a.name).truncate().weight(700).color(fg(0.95))).child(label(kind).caption().truncate()));
    if g.ui.confirm.as_ref() == Some(&Confirm::Account(a.id.clone())) {
        let del = id.clone();
        let confirm = row()
            .gap(8.0)
            .child(label(tr!("accounts.delete_confirm")).font_size(13.0).color(RED))
            .child(button(tr!("common.delete")).on_click(move |g: &mut Gui| {
                g.ui.confirm = None;
                g.remove_after_collapse(Removal::Account(del.clone()));
            }))
            .child(button(tr!("common.cancel")).on_click(|g: &mut Gui| g.ui.confirm = None));
        return r.child(swap(("confirm", a.id.as_str()), confirm)).into();
    }
    r = r.child(action_slot(90.0, a.selected.then(|| tag(tr!("accounts.current")).accent().into())));
    if let AccountKind::Yggdrasil = a.kind {
        let is_littleskin = a.yggdrasil_server.as_deref().unwrap_or("").contains("littleskin");
        r = r.child(tag(if is_littleskin { "LittleSkin" } else { tr!("accounts.yggdrasil") }));
    }
    r.child(icon_button(Icon::Trash).label(tr!("accounts.remove")).on_click(move |g: &mut Gui| g.ui.confirm = Some(Confirm::Account(id.clone())))).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_bridge::{Appearance, Bridge, Language, Sink, ThemeMode};
    use cairn_ui::testing::Headless;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    #[test]
    fn microsoft_login_success_animates_and_can_be_dismissed() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let mut text = Some(cairn_ui::TextSystem::new(&cairn_ui::testing::system_font_setup()));
        for (language, light, reduced) in
            [(Language::ZhCn, false, false), (Language::EnUs, true, false), (Language::JaJp, false, false), (Language::ZhCn, false, true)]
        {
            cairn_i18n::with_language(language, || {
                let sink: Sink = Arc::new(|_| true);
                let mut gui = Gui::new(Bridge::start(rt.handle().clone(), sink));
                gui.m.ms_available = true;
                gui.ui.add_account_open = true;
                gui.ui.add_account_tab = AddAccountTab::Microsoft;
                gui.m.ms_login = MsLogin::Active {
                    user_code: "TEST-1234".into(),
                    verification_uri: "https://www.microsoft.com/link".into(),
                    deadline: Instant::now() + Duration::from_secs(900),
                    status: DeviceLoginStatus::WaitingForUser,
                    code_copied: true,
                };
                let appearance = Appearance { mode: if light { ThemeMode::Light } else { ThemeMode::Dark }, ..Appearance::default() };
                let mut theme = crate::style::theme(&appearance, None, false);
                if reduced {
                    theme.motion = theme.motion.reduced();
                }
                let view = |g: &Gui| column().fill_width().fill_height().padding(20.0).align(Align::Stretch).child(add_account_popover(g));
                let mut app = Headless::with_theme(gui, view, 480.0, 500.0, 1.0, theme, text.take().unwrap());
                app.render();
                assert!(app.find(tr!("auth.ms.code_copied")).is_some());
                assert!(app.find(tr!("auth.ms.bottom_tip")).unwrap().bounds.bottom() <= 480.0);
                let preview = std::env::var_os("CAIRN_AUTH_PREVIEW_DIR").map(std::path::PathBuf::from);
                let snapshot = |app: &Headless<Gui>, phase: &str| {
                    if let Some(dir) = &preview {
                        std::fs::create_dir_all(dir).unwrap();
                        std::fs::write(dir.join(format!("{}-{light}-{reduced}-{phase}.png", language.code())), app.png()).unwrap();
                    }
                };
                snapshot(&app, "waiting");
                let code_bounds = app.find("TEST-1234").unwrap().bounds;
                let tip_bounds = app.find(tr!("auth.ms.bottom_tip")).unwrap().bounds;
                app.update(|g| {
                    if let MsLogin::Active { code_copied, .. } = &mut g.m.ms_login {
                        *code_copied = false;
                    }
                });
                assert!(app.find(tr!("auth.ms.copy_code")).is_some());
                assert!(app.find(tr!("auth.ms.code_copied")).is_none());
                assert_eq!(app.find("TEST-1234").unwrap().bounds, code_bounds);
                assert_eq!(app.find(tr!("auth.ms.bottom_tip")).unwrap().bounds, tip_bounds);
                app.update(|g| g.m.ms_login = MsLogin::Succeeded { account_id: "test".into(), name: "CairnTest".into() });
                app.render();
                if reduced {
                    app.run_animations(1);
                }
                assert_eq!(app.animating(), !reduced);
                app.run_animations(3);
                snapshot(&app, "entering");
                app.run_animations(60);
                assert!(!app.animating());
                assert!(app.find(tr!("auth.ms.success_title")).is_some());
                assert!(app.find("TEST-1234").is_none());
                let done = app.find(tr!("auth.ms.done")).unwrap();
                assert!(done.bounds.right() <= 460.0 && done.bounds.bottom() <= 480.0);
                snapshot(&app, "success");
                app.click(done.bounds.center());
                assert!(!app.state().ui.add_account_open);
                app.update(|g| g.open_add_account());
                assert!(matches!(app.state().m.ms_login, MsLogin::Idle));
                text = Some(app.into_text_system());
            });
        }
    }

    fn account(id: &str, kind: AccountKind) -> AccountInfo {
        AccountInfo { id: id.into(), name: id.into(), kind, uuid: String::new(), skin_url: None, selected: false, needs_refresh: false, yggdrasil_server: None }
    }

    #[test]
    fn add_menu_opens_on_microsoft_and_locks_the_rest_until_allowed() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        cairn_i18n::with_language(Language::ZhCn, || {
            let sink: Sink = Arc::new(|_| true);
            let mut gui = Gui::new(Bridge::start(rt.handle().clone(), sink));
            gui.m.accounts = Remote::Ready(Vec::new());
            gui.ui.add_account_tab = AddAccountTab::Offline;
            gui.open_add_account();
            assert_eq!(gui.ui.add_account_tab, AddAccountTab::Microsoft, "每次打开都从微软账号开始");

            let theme = crate::style::theme(&Appearance::default(), None, false);
            let view = |g: &Gui| column().fill_width().fill_height().padding(20.0).align(Align::Stretch).child(add_account_popover(g));
            let text = cairn_ui::TextSystem::new(&cairn_ui::testing::system_font_setup());
            let mut app = Headless::with_theme(gui, view, 560.0, 560.0, 1.0, theme, text);
            app.render();
            let x = |app: &Headless<Gui>, name: &str| app.find(name).expect(name).bounds.x;
            let (ms, offline, ygg) = (x(&app, tr!("accounts.microsoft")), x(&app, tr!("accounts.offline")), x(&app, tr!("accounts.yggdrasil")));
            assert!(ms < offline && offline < ygg, "标签的顺序:微软 {ms}、离线 {offline}、外置认证 {ygg}");

            // Outside mainland China with no Microsoft account, offline and external auth show only a hint.
            for tab in [AddAccountTab::Offline, AddAccountTab::Yggdrasil] {
                app.update(move |g| g.ui.add_account_tab = tab);
                app.render();
                assert!(app.find(tr!("accounts.locked_title")).is_some(), "{tab:?}");
                assert!(app.find(tr!("accounts.offline_feature_hint")).is_none() && app.find(tr!("accounts.ygg_authlib_hint")).is_none(), "{tab:?}");
            }
            // The hint's button switches back to the Microsoft tab.
            let go = app.find(tr!("accounts.locked_action")).expect("按钮").bounds.center();
            app.click(go);
            assert_eq!(app.state().ui.add_account_tab, AddAccountTab::Microsoft);

            // In mainland China, offline accounts can be added.
            app.update(|g| {
                g.m.mainland_china = true;
                g.ui.add_account_tab = AddAccountTab::Offline;
            });
            app.render();
            assert!(app.find(tr!("accounts.locked_title")).is_none());
            assert!(app.find(tr!("accounts.offline_feature_hint")).is_some());
            // Outside mainland China with a Microsoft account, offline and external auth can be added too.
            app.update(|g| {
                g.m.mainland_china = false;
                g.m.accounts = Remote::Ready(vec![account("ms", AccountKind::Microsoft)]);
                g.ui.add_account_tab = AddAccountTab::Yggdrasil;
            });
            app.render();
            assert!(app.find(tr!("accounts.locked_title")).is_none());
            assert!(app.find(tr!("accounts.ygg_authlib_hint")).is_some());
            // Offline accounts alone do not count. A Microsoft account is still required.
            app.update(|g| g.m.accounts = Remote::Ready(vec![account("off", AccountKind::Offline)]));
            app.render();
            assert!(app.find(tr!("accounts.locked_title")).is_some());
        });
    }
}

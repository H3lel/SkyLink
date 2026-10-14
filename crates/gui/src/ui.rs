//! GTK4 front end: Home (connect), News, Feedback, About — the same pages as
//! the Windows client.

use crate::engine::{self, CoreMsg, Elevation, Mode};
use gtk::prelude::*;
use gtk::glib;
use nthlink_common::model::{DirectoryServerConfig, FeedbackParameter};
use nthlink_common::paths;
use nthlink_common::rootgo::Bridge;
use nthlink_common::state::{self, ClientState, Settings};
use nthlink_common::tunnel::CoreEvent;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;

const APP_ID: &str = "com.nthlink.linux";
const ISSUE_CATEGORIES: &[&str] = &["General feedback", "Cannot connect", "Connection speed is slow", "Suggestions", "Other"];

const CSS: &str = r#"
window.nthlink { background: #0d1530; color: #e8ecf8; }
window.nthlink headerbar { background: #0a1128; color: #e8ecf8; box-shadow: none; border: none; }
.page { padding: 18px; }
.status-title { font-size: 22px; font-weight: 700; }
.hint { color: #9aa6c7; }
.detail { color: #9aa6c7; font-size: 12px; }
.error-text { color: #ff8a80; }
.ok-text { color: #69f0ae; }
button.connect {
  min-width: 170px; min-height: 170px; border-radius: 999px;
  font-size: 20px; font-weight: 700; color: #ffffff;
  background: radial-gradient(circle, #2b6cff 0%, #1640b8 100%);
  border: 6px solid rgba(255,255,255,0.12);
}
button.connect.connected { background: radial-gradient(circle, #19c37d 0%, #0b7a4b 100%); }
button.connect.busy { background: radial-gradient(circle, #6b7799 0%, #3c4565 100%); }
.card { background: rgba(255,255,255,0.05); border-radius: 12px; padding: 12px; }
list.news { background: transparent; }
list.news row { padding: 10px 6px; border-bottom: 1px solid rgba(255,255,255,0.07); }
.news-title { font-weight: 700; }
textview.feedback, textview.feedback text { background: rgba(255,255,255,0.06); color: #e8ecf8; }
"#;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Phase {
    Disconnected,
    Connecting,
    Connected,
    Disconnecting,
}

enum UiMsg {
    Progress(String),
    PrepareFailed(String),
    Started { directory: DirectoryServerConfig, summary: String, from_cache: bool, stop: mpsc::Sender<CoreMsg> },
    Core(CoreMsg),
    Health(bool),
    Feedback(Result<(), String>),
}

struct Widgets {
    window: gtk::ApplicationWindow,
    status: gtk::Label,
    hint: gtk::Label,
    detail: gtk::Label,
    health: gtk::Label,
    button: gtk::Button,
    landing: gtk::Button,
    notifications: gtk::Box,
    news: gtk::ListBox,
    news_empty: gtk::Label,
    fb_send: gtk::Button,
    fb_status: gtk::Label,
}

struct App {
    w: Widgets,
    phase: Phase,
    settings: Settings,
    directory: Option<DirectoryServerConfig>,
    stop: Option<mpsc::Sender<CoreMsg>>,
    tx: async_channel::Sender<UiMsg>,
    quit_when_stopped: bool,
    health_timer: Option<glib::SourceId>,
    landing_opened: bool,
}

pub fn run() -> glib::ExitCode {
    let app = gtk::Application::builder().application_id(APP_ID).build();
    app.connect_activate(build);
    // GTK must not see our own subcommand arguments.
    app.run_with_args::<&str>(&[])
}

fn label(text: &str, classes: &[&str]) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    for c in classes {
        l.add_css_class(c);
    }
    l.set_wrap(true);
    l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    l.set_justify(gtk::Justification::Center);
    l
}

fn is_rtl(text: &str) -> bool {
    text.chars().find(|c| c.is_alphabetic()).map(|c| matches!(c as u32, 0x0590..=0x08FF | 0xFB1D..=0xFEFC)).unwrap_or(false)
}

fn build(app: &gtk::Application) {
    if let Some(win) = app.active_window() {
        win.present();
        return;
    }
    let provider = gtk::CssProvider::new();
    #[allow(deprecated)]
    provider.load_from_data(CSS);
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        if let Some(res) = paths::resource_dir() {
            gtk::IconTheme::for_display(&display).add_search_path(res.join("icons"));
        }
    }
    gtk::Window::set_default_icon_name("nthlink");

    let window = gtk::ApplicationWindow::builder().application(app).title("nthLink").default_width(440).default_height(680).build();
    window.add_css_class("nthlink");

    let stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).build();
    let switcher = gtk::StackSwitcher::builder().stack(&stack).build();
    let header = gtk::HeaderBar::new();
    header.set_title_widget(Some(&switcher));
    window.set_titlebar(Some(&header));
    window.set_child(Some(&stack));

    let client = ClientState::load();

    // ---------- Home ----------
    let home = gtk::Box::new(gtk::Orientation::Vertical, 14);
    home.add_css_class("page");
    home.set_valign(gtk::Align::Start);
    if let Some(res) = paths::resource_dir() {
        let logo = res.join("icons").join("nthlink.png");
        if logo.is_file() {
            let pic = gtk::Picture::for_filename(&logo);
            pic.set_can_shrink(true);
            pic.set_size_request(-1, 72);
            home.append(&pic);
        }
    }
    let status = label("Not connected", &["status-title"]);
    let button = gtk::Button::with_label("Connect");
    button.add_css_class("connect");
    button.set_halign(gtk::Align::Center);
    button.set_margin_top(12);
    button.set_margin_bottom(6);
    let hint = label("Tap To Connect", &["hint"]);
    let detail = label("", &["detail"]);
    detail.set_selectable(true);
    let health = label("", &["detail"]);
    let landing = gtk::Button::with_label("Landing Page");
    landing.set_halign(gtk::Align::Center);
    landing.set_visible(false);
    let notifications = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let auto_landing = gtk::CheckButton::with_label("Open the landing page after connecting");
    auto_landing.set_active(client.settings.open_landing_page);
    auto_landing.set_halign(gtk::Align::Center);
    for wdg in [status.upcast_ref::<gtk::Widget>(), button.upcast_ref(), hint.upcast_ref(), detail.upcast_ref(), health.upcast_ref(), landing.upcast_ref(), notifications.upcast_ref(), auto_landing.upcast_ref()] {
        home.append(wdg);
    }
    stack.add_titled(&home, Some("home"), "Home");

    // ---------- News ----------
    let news_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
    news_box.add_css_class("page");
    let news_empty = label("News appears here after the first connection.", &["hint"]);
    let news = gtk::ListBox::new();
    news.add_css_class("news");
    news.set_selection_mode(gtk::SelectionMode::None);
    news.set_activate_on_single_click(true);
    news.connect_row_activated(|_, row| {
        let url = row.widget_name();
        engine::open_url(url.as_str());
    });
    let scroller = gtk::ScrolledWindow::builder().child(&news).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();
    news_box.append(&news_empty);
    news_box.append(&scroller);
    stack.add_titled(&news_box, Some("news"), "News");

    // ---------- Feedback ----------
    let fb = gtk::Box::new(gtk::Orientation::Vertical, 8);
    fb.add_css_class("page");
    let fb_title = gtk::Label::new(Some("Issue category"));
    fb_title.set_xalign(0.0);
    let category = gtk::DropDown::from_strings(ISSUE_CATEGORIES);
    let email_l = gtk::Label::new(Some("Email"));
    email_l.set_xalign(0.0);
    let email = gtk::Entry::new();
    email.set_placeholder_text(Some("optional"));
    let desc_l = gtk::Label::new(Some("Description"));
    desc_l.set_xalign(0.0);
    let desc = gtk::TextView::new();
    desc.add_css_class("feedback");
    desc.set_wrap_mode(gtk::WrapMode::WordChar);
    let desc_scroll = gtk::ScrolledWindow::builder().child(&desc).min_content_height(160).vexpand(true).build();
    let note = label(
        "Your description, email address (if provided), and additional information will be sent to the nthLink team. Please refer to the nthLink website's Privacy Policy for our data collection policy.",
        &["detail"],
    );
    let fb_send = gtk::Button::with_label("Send");
    fb_send.set_sensitive(false);
    let fb_status = label("", &["detail"]);
    for wdg in [fb_title.upcast_ref::<gtk::Widget>(), category.upcast_ref(), email_l.upcast_ref(), email.upcast_ref(), desc_l.upcast_ref(), desc_scroll.upcast_ref(), note.upcast_ref(), fb_send.upcast_ref(), fb_status.upcast_ref()] {
        fb.append(wdg);
    }
    stack.add_titled(&fb, Some("feedback"), "Feedback");
    {
        let fb_send = fb_send.clone();
        desc.buffer().connect_changed(move |b| fb_send.set_sensitive(b.char_count() > 0));
    }

    // ---------- About ----------
    let about = gtk::Box::new(gtk::Orientation::Vertical, 10);
    about.add_css_class("page");
    about.append(&label(&format!("nthLink for Linux — version {}", env!("CARGO_PKG_VERSION")), &["status-title"]));
    about.append(&label(
        "We are a group of experienced software and information security engineers who started nthLink in 2016 to support human rights lawyers who needed to obtain restricted information and to express their perspectives. Our service is free and will remain free thanks to our sponsors and partners.",
        &["hint"],
    ));
    let cid = label(&format!("Client ID: {}", client.client_guid), &["detail"]);
    cid.set_selectable(true);
    about.append(&cid);
    about.append(&label(&format!("Logs: {}", paths::log_dir().display()), &["detail"]));
    let links = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    links.set_halign(gtk::Align::Center);
    let site = gtk::LinkButton::with_label("https://www.nthlink.com", "nthlink.com");
    let open_logs = gtk::Button::with_label("Open logs folder");
    open_logs.connect_clicked(|_| {
        let _ = paths::ensure_private_dir(&paths::log_dir());
        engine::open_url(&format!("file://{}", paths::log_dir().display()));
    });
    links.append(&site);
    links.append(&open_logs);
    about.append(&links);
    let reset = gtk::Button::with_label("Reset Wine environment");
    reset.set_halign(gtk::Align::Center);
    reset.set_tooltip_text(Some("Deletes the private Wine prefix used to reach the directory service; it is recreated on the next connection."));
    let reset_status = label("", &["detail"]);
    {
        let reset_status = reset_status.clone();
        reset.connect_clicked(move |_| {
            if let Ok(b) = Bridge::locate() {
                b.kill_wineserver();
            }
            let p = paths::data_dir().join("wineprefix");
            let msg = match std::fs::remove_dir_all(&p) {
                Ok(()) => "Wine environment removed.".to_string(),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => "Nothing to reset.".to_string(),
                Err(e) => format!("Reset failed: {e}"),
            };
            reset_status.set_text(&msg);
        });
    }
    about.append(&reset);
    about.append(&reset_status);
    stack.add_titled(&about, Some("about"), "About");

    let (tx, rx) = async_channel::unbounded::<UiMsg>();
    let app_state = Rc::new(RefCell::new(App {
        w: Widgets { window: window.clone(), status, hint, detail, health, button: button.clone(), landing: landing.clone(), notifications, news, news_empty, fb_send: fb_send.clone(), fb_status },
        phase: Phase::Disconnected,
        settings: client.settings.clone(),
        directory: None,
        stop: None,
        tx: tx.clone(),
        quit_when_stopped: false,
        health_timer: None,
        landing_opened: false,
    }));

    // Show news/notifications from the last session right away.
    if let Some(cfg) = ClientState::load_last_config() {
        app_state.borrow_mut().show_directory(&cfg);
    }

    {
        let st = app_state.clone();
        auto_landing.connect_toggled(move |c| {
            let mut s = st.borrow_mut();
            s.settings.open_landing_page = c.is_active();
            let mut cs = ClientState::load();
            cs.settings = s.settings.clone();
            cs.save_settings();
        });
    }
    {
        let st = app_state.clone();
        button.connect_clicked(move |_| App::toggle(&st));
    }
    {
        let st = app_state.clone();
        landing.connect_clicked(move |_| {
            if let Some(d) = &st.borrow().directory {
                engine::open_url(&d.redirect_url);
            }
        });
    }
    {
        let st = app_state.clone();
        let desc = desc.clone();
        let email = email.clone();
        let category = category.clone();
        fb_send.connect_clicked(move |b| {
            let buf = desc.buffer();
            let text = buf.text(&buf.start_iter(), &buf.end_iter(), false).to_string();
            if text.trim().is_empty() {
                return;
            }
            b.set_sensitive(false);
            let s = st.borrow();
            s.w.fb_status.set_text("Sending…");
            let feedback_type = ISSUE_CATEGORIES.get(category.selected() as usize).copied().unwrap_or("General feedback").to_string();
            let email = email.text().to_string();
            let tx = s.tx.clone();
            std::thread::spawn(move || {
                let cs = ClientState::load();
                let r = Bridge::locate().and_then(|b| {
                    b.feedback(
                        &cs.data,
                        &FeedbackParameter {
                            client_id: cs.client_guid.clone(),
                            language: state::culture_name(),
                            os: cs.settings.identity.os.clone(),
                            app_version: cs.settings.identity.app_version.clone(),
                            utc_sent: state::utc_now_string(),
                            feedback_type,
                            description: text,
                            error_code: None,
                            error_message: None,
                            email,
                        },
                    )
                });
                let _ = tx.send_blocking(UiMsg::Feedback(r));
            });
        });
    }
    {
        let st = app_state.clone();
        let desc = desc.clone();
        glib::spawn_future_local(async move {
            while let Ok(msg) = rx.recv().await {
                if let UiMsg::Feedback(Ok(())) = &msg {
                    desc.buffer().set_text("");
                }
                App::handle(&st, msg);
            }
        });
    }
    {
        let st = app_state.clone();
        window.connect_close_request(move |w| {
            let mut s = st.borrow_mut();
            match s.phase {
                Phase::Disconnected => glib::Propagation::Proceed,
                Phase::Connected | Phase::Connecting => {
                    s.quit_when_stopped = true;
                    s.request_stop();
                    w.set_visible(false);
                    glib::Propagation::Stop
                }
                Phase::Disconnecting => {
                    s.quit_when_stopped = true;
                    w.set_visible(false);
                    glib::Propagation::Stop
                }
            }
        });
    }

    window.present();
}

impl App {
    fn toggle(st: &Rc<RefCell<App>>) {
        let mut s = st.borrow_mut();
        match s.phase {
            Phase::Disconnected => s.start(),
            Phase::Connected | Phase::Connecting => s.request_stop(),
            Phase::Disconnecting => {}
        }
    }

    fn start(&mut self) {
        self.set_phase(Phase::Connecting);
        self.landing_opened = false;
        self.set_detail("", false);
        let tx = self.tx.clone();
        let settings = self.settings.clone();
        std::thread::spawn(move || {
            let mut cs = ClientState::load();
            cs.settings = settings;
            let progress = |m: &str| {
                let _ = tx.send_blocking(UiMsg::Progress(m.to_string()));
            };
            let prep = match engine::prepare(&mut cs, &progress) {
                Ok(p) => p,
                Err(e) => {
                    let _ = tx.send_blocking(UiMsg::PrepareFailed(e));
                    return;
                }
            };
            progress("Waiting for administrator authorization…");
            let (ctx, crx) = mpsc::channel();
            let elevation = if unsafe { libc::geteuid() } == 0 { Elevation::None } else { Elevation::Pkexec };
            let handle = match engine::spawn_core(&prep, Mode::Tun, elevation, ctx.clone()) {
                Ok(h) => h,
                Err(e) => {
                    let _ = tx.send_blocking(UiMsg::PrepareFailed(e));
                    return;
                }
            };
            let _ = tx.send_blocking(UiMsg::Started {
                directory: prep.directory.clone(),
                summary: prep.leaf.summary.clone(),
                from_cache: prep.from_cache,
                stop: ctx,
            });
            let ftx = tx.clone();
            engine::supervise(handle, crx, move |m| {
                let _ = ftx.send_blocking(UiMsg::Core(m));
            });
        });
    }

    fn request_stop(&mut self) {
        if let Some(stop) = &self.stop {
            let _ = stop.send(CoreMsg::StopRequested);
            self.set_phase(Phase::Disconnecting);
        }
        // While still preparing there is no core yet; the Started handler
        // below notices the phase and stops it immediately.
        else if self.phase == Phase::Connecting {
            self.set_phase(Phase::Disconnecting);
        }
    }

    fn set_detail(&self, text: &str, error: bool) {
        let d = &self.w.detail;
        d.set_text(text);
        if error {
            d.add_css_class("error-text");
        } else {
            d.remove_css_class("error-text");
        }
    }

    fn set_phase(&mut self, p: Phase) {
        self.phase = p;
        let w = &self.w;
        for c in ["connected", "busy"] {
            w.button.remove_css_class(c);
        }
        let (status, btn, hint) = match p {
            Phase::Disconnected => ("Not connected", "Connect", "Tap To Connect"),
            Phase::Connecting => ("Connecting…", "Cancel", ""),
            Phase::Connected => ("Connected", "Disconnect", "Tap To Disconnect"),
            Phase::Disconnecting => ("Disconnecting…", "…", ""),
        };
        match p {
            Phase::Connected => w.button.add_css_class("connected"),
            Phase::Connecting | Phase::Disconnecting => w.button.add_css_class("busy"),
            Phase::Disconnected => {}
        }
        w.status.set_text(status);
        w.button.set_label(btn);
        w.button.set_sensitive(p != Phase::Disconnecting);
        w.hint.set_text(hint);
        w.landing.set_visible(p == Phase::Connected && self.directory.as_ref().map(|d| !d.redirect_url.is_empty()).unwrap_or(false));
        if p != Phase::Connected {
            w.health.set_text("");
            if let Some(id) = self.health_timer.take() {
                id.remove();
            }
        }
    }

    fn show_directory(&mut self, cfg: &DirectoryServerConfig) {
        let w = &self.w;
        while let Some(c) = w.news.first_child() {
            w.news.remove(&c);
        }
        let mut items: Vec<_> = cfg.headline_news.iter().collect();
        items.sort_by_key(|n| !n.pin_to_top);
        for n in &items {
            let row = gtk::ListBoxRow::new();
            row.set_widget_name(&n.url);
            let b = gtk::Box::new(gtk::Orientation::Vertical, 4);
            let title = gtk::Label::new(Some(&n.title));
            title.add_css_class("news-title");
            title.set_wrap(true);
            title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            let rtl = is_rtl(&n.title);
            title.set_xalign(if rtl { 1.0 } else { 0.0 });
            b.append(&title);
            if !n.excerpt.is_empty() {
                let ex = gtk::Label::new(Some(&n.excerpt));
                ex.add_css_class("detail");
                ex.set_wrap(true);
                ex.set_xalign(if is_rtl(&n.excerpt) { 1.0 } else { 0.0 });
                b.append(&ex);
            }
            row.set_child(Some(&b));
            w.news.append(&row);
        }
        w.news_empty.set_visible(items.is_empty());

        while let Some(c) = w.notifications.first_child() {
            w.notifications.remove(&c);
        }
        for n in &cfg.notifications {
            if n.url.is_empty() {
                w.notifications.append(&label(&n.title, &["hint"]));
            } else {
                let lb = gtk::LinkButton::with_label(&n.url, &n.title);
                w.notifications.append(&lb);
            }
        }
        self.directory = Some(cfg.clone());
    }

    fn start_health_checks(st: &Rc<RefCell<App>>) {
        let tx = st.borrow().tx.clone();
        let check = move || {
            let tx = tx.clone();
            std::thread::spawn(move || {
                let _ = tx.send_blocking(UiMsg::Health(engine::health_check()));
            });
        };
        check();
        let id = glib::timeout_add_seconds_local(30, move || {
            check();
            glib::ControlFlow::Continue
        });
        st.borrow_mut().health_timer = Some(id);
    }

    fn handle(st: &Rc<RefCell<App>>, msg: UiMsg) {
        match msg {
            UiMsg::Progress(m) => {
                let s = st.borrow();
                if s.phase == Phase::Connecting {
                    s.set_detail(&m, false);
                }
            }
            UiMsg::PrepareFailed(e) => {
                let mut s = st.borrow_mut();
                s.set_phase(Phase::Disconnected);
                s.set_detail(&e, true);
                s.finish_quit_if_requested();
            }
            UiMsg::Started { directory, summary, from_cache, stop } => {
                let mut s = st.borrow_mut();
                s.show_directory(&directory);
                s.stop = Some(stop);
                if s.phase == Phase::Disconnecting {
                    // Cancel was pressed while preparing.
                    s.request_stop();
                } else {
                    s.set_detail(&format!("Starting tunnel ({summary}{})…", if from_cache { ", cached" } else { "" }), false);
                }
            }
            UiMsg::Core(CoreMsg::Event(ev)) => match ev {
                CoreEvent::Starting => st.borrow().set_detail("Starting tunnel…", false),
                CoreEvent::Connected { ipv6, dns } => {
                    {
                        let mut s = st.borrow_mut();
                        s.set_phase(Phase::Connected);
                        s.set_detail(&format!("DNS via {dns} · IPv6 {}", if ipv6 { "protected" } else { "not routed" }), false);
                        s.w.health.set_text("Checking connection…");
                        if s.settings.open_landing_page && !s.landing_opened {
                            s.landing_opened = true;
                            if let Some(d) = &s.directory {
                                let url = d.redirect_url.clone();
                                glib::timeout_add_seconds_local_once(1, move || engine::open_url(&url));
                            }
                        }
                    }
                    App::start_health_checks(st);
                }
                CoreEvent::Error { message } => st.borrow().set_detail(&message, true),
                CoreEvent::Log { level, message } => {
                    if level == "warn" {
                        let s = st.borrow();
                        let cur = s.w.detail.text();
                        s.set_detail(&format!("{cur}\n{message}"), false);
                    }
                }
                CoreEvent::Stopped => {}
            },
            UiMsg::Core(CoreMsg::Exited(code, stderr)) => {
                let mut s = st.borrow_mut();
                let was_error = s.w.detail.has_css_class("error-text");
                s.stop = None;
                s.set_phase(Phase::Disconnected);
                if let Some(m) = engine::explain_exit(code, &stderr) {
                    s.set_detail(&m, true);
                } else if !was_error {
                    s.set_detail("", false);
                }
                s.finish_quit_if_requested();
            }
            UiMsg::Core(CoreMsg::StopRequested) => {}
            UiMsg::Health(ok) => {
                let s = st.borrow();
                if s.phase == Phase::Connected {
                    let h = &s.w.health;
                    h.remove_css_class("ok-text");
                    h.remove_css_class("error-text");
                    if ok {
                        h.set_text("● Connection healthy");
                        h.add_css_class("ok-text");
                    } else {
                        h.set_text("● Connection check failed — retrying");
                        h.add_css_class("error-text");
                    }
                }
            }
            UiMsg::Feedback(r) => {
                let s = st.borrow();
                s.w.fb_send.set_sensitive(true);
                match r {
                    Ok(()) => s.w.fb_status.set_text("Thanks for helping us improve! We love hearing from you."),
                    Err(e) => s.w.fb_status.set_text(&format!("Can't connect to nthlink server. ({e})")),
                }
            }
        }
    }

    fn finish_quit_if_requested(&self) {
        if self.quit_when_stopped {
            if let Some(app) = self.w.window.application() {
                app.quit();
            }
        }
    }
}

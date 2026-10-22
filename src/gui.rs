//! The window. It runs as the user, keeps the config, and drives the root helper.
//! Opening it asks for access, and picking Player 2's keyboard starts play: no Start step.

use crate::layout::{key_name, shared_with_player_1, Config, Layout, ACTIONS, OUTPUT_GROUPS, TOGGLE};
use crate::protocol::{self, Command, Event, Keyboard, Mode};
use eframe::egui::{self, Color32, CornerRadius, FontId, Margin, RichText, Stroke, StrokeKind, TextStyle};
use evdev::KeyCode as K;
use std::{
    collections::HashSet,
    io::{self, BufRead, BufReader},
    process::{Child, ChildStdin, Command as Process, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

// GGST paints Controller 1 red and Controller 2 blue; couch-strive is Player 2's tool.
const P2: Color32 = Color32::from_rgb(61, 139, 242);
const P1: Color32 = Color32::from_rgb(214, 52, 64);
const AMBER: Color32 = Color32::from_rgb(240, 169, 60);
const INK: Color32 = Color32::from_rgb(16, 21, 31);
const PANEL: Color32 = Color32::from_rgb(23, 32, 49);
const LINE: Color32 = Color32::from_rgb(38, 50, 74);
const CAP: Color32 = Color32::from_rgb(34, 45, 66);
const CAP_HOVER: Color32 = Color32::from_rgb(46, 60, 87);
const CAP_EDGE: Color32 = Color32::from_rgb(10, 14, 22);
const TEXT: Color32 = Color32::from_rgb(233, 238, 248);
const MUTED: Color32 = Color32::from_rgb(141, 151, 171);
/// A tapped key stays lit at least this long, so a quick tap still shows.
const MIN_LIT: f64 = 0.08;

pub fn run() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("couch-strive")
            .with_app_id("couch-strive")
            .with_inner_size([560.0, 540.0])
            .with_min_inner_size([500.0, 460.0]),
        ..Default::default()
    };
    eframe::run_native("couch-strive", options, Box::new(|cc| Ok(Box::new(App::new(cc.egui_ctx.clone())))))
}

fn style(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|s| {
        s.text_styles.insert(TextStyle::Body, FontId::proportional(14.0));
        s.text_styles.insert(TextStyle::Button, FontId::proportional(14.0));
        s.text_styles.insert(TextStyle::Small, FontId::proportional(12.0));
        s.text_styles.insert(TextStyle::Monospace, FontId::monospace(13.0));
        s.spacing.item_spacing = egui::vec2(6.0, 6.0);
        s.spacing.button_padding = egui::vec2(8.0, 3.0);
        // the first tooltip waits a little; its neighbours then open at once
        s.interaction.tooltip_delay = 0.3;
        s.interaction.tooltip_grace_time = 0.4;
        let v = &mut s.visuals;
        v.panel_fill = INK;
        v.window_fill = PANEL;
        v.window_stroke = Stroke::new(1.0, LINE);
        v.override_text_color = Some(TEXT);
        v.selection.bg_fill = P2;
        v.widgets.inactive.weak_bg_fill = CAP;
        v.widgets.inactive.bg_fill = CAP;
        v.widgets.hovered.weak_bg_fill = CAP_HOVER;
        v.widgets.hovered.bg_fill = CAP_HOVER;
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, P2);
        v.widgets.active.weak_bg_fill = LINE;
        v.widgets.active.bg_fill = LINE;
        for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
            w.corner_radius = CornerRadius::same(6);
        }
    });
}

/// The root helper, started through pkexec. Dropping this closes its stdin, which makes it clean up and exit.
struct Helper {
    child: Child,
    stdin: ChildStdin,
    events: mpsc::Receiver<Event>,
}

impl Helper {
    fn start(ctx: egui::Context) -> io::Result<Helper> {
        let exe = std::env::current_exe()?;
        // SAFETY: geteuid has no preconditions.
        let mut cmd = if unsafe { libc::geteuid() } == 0 {
            Process::new(exe)
        } else {
            let mut pkexec = Process::new("pkexec");
            pkexec.arg(exe);
            pkexec
        };
        let mut child = cmd.arg("helper").stdin(Stdio::piped()).stdout(Stdio::piped()).spawn()?;
        let stdin = child.stdin.take().expect("stdin is piped");
        let stdout = child.stdout.take().expect("stdout is piped");
        let (tx, events) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let event = protocol::parse(&line)
                    .unwrap_or_else(|e| Event::Error { message: format!("Unreadable message from the helper: {e}") });
                if tx.send(event).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
            drop(tx); // lets the window see that the helper is gone
            ctx.request_repaint();
        });
        Ok(Helper { child, stdin, events })
    }
}

enum Access {
    /// pkexec is showing the password prompt.
    Asking,
    Granted,
    Denied(String),
}

struct Checks {
    keyd_running: bool,
    virtual_keyboard_error: Option<String>,
}

/// The one thing the status card's button does.
#[derive(Clone, Copy)]
enum Act {
    Retry,
    PauseKeyd,
    Pause,
    Resume,
}

struct App {
    ctx: egui::Context,
    config: Config,
    layout: Layout,
    helper: Option<Helper>,
    access: Access,
    checks: Option<Checks>,
    keyboards: Vec<Keyboard>,
    watching: Option<String>,
    mode: Mode,
    /// Set by the Pause button, so picking or reconnecting a keyboard doesn't resume on its own.
    paused: bool,
    /// The action waiting for Player 2 to press its new key.
    rebinding: Option<usize>,
    /// Keys Player 2 is holding right now, which light up as pressed.
    held: HashSet<K>,
    /// Player 2's last key press and when, so a quick tap still shows.
    last_press: Option<(K, f64)>,
    /// The last key clicked away, for Undo.
    removed: Option<(usize, K)>,
    note: String,
    first_run: bool,
}

impl App {
    fn new(ctx: egui::Context) -> Self {
        style(&ctx);
        let (config, note) = match Config::load() {
            Ok(config) => (config, String::new()),
            Err(e) => (Config::default(), format!("Couldn't read the saved layout, so the defaults are in use: {e}")),
        };
        let mut app = App {
            ctx,
            layout: config.layout(),
            first_run: config.keyboard_id.is_none(),
            config,
            helper: None,
            access: Access::Asking,
            checks: None,
            keyboards: Vec::new(),
            watching: None,
            mode: Mode::Stopped,
            paused: false,
            rebinding: None,
            held: HashSet::new(),
            last_press: None,
            removed: None,
            note,
        };
        match std::env::var("COUCH_STRIVE_DEMO") {
            Ok(state) => app.demo(&state),
            Err(_) => app.start_helper(),
        }
        app
    }

    /// Fake helper states for screenshots and design reviews, without root or keyboards.
    fn demo(&mut self, state: &str) {
        let keyboard = |id: &str, name: &str| Keyboard { id: id.into(), name: name.into(), blocked: None };
        self.access = Access::Granted;
        self.keyboards = vec![keyboard("a", "BY Tech Gaming Keyboard"), keyboard("b", "Logitech USB Keyboard")];
        self.checks = Some(Checks { keyd_running: state == "keyd", virtual_keyboard_error: None });
        if state == "playing" {
            self.watching = Some("b".into());
            self.mode = Mode::Game;
            self.held.insert(K::KEY_I);
            self.rebinding = Some(9);
        }
    }

    fn start_helper(&mut self) {
        self.access = Access::Asking;
        match Helper::start(self.ctx.clone()) {
            Ok(helper) => self.helper = Some(helper),
            Err(e) => self.access = Access::Denied(format!("Couldn't start the helper: {e}.")),
        }
    }

    /// Shows a status line; any new message retires the Undo offer.
    fn say(&mut self, text: impl Into<String>) {
        self.note = text.into();
        self.removed = None;
    }

    fn send(&mut self, cmd: Command) {
        if let Some(helper) = &mut self.helper
            && let Err(e) = protocol::send(&mut helper.stdin, &cmd)
        {
            self.say(format!("Couldn't reach the helper: {e}"));
        }
    }

    fn save(&mut self) {
        if let Err(e) = self.config.save() {
            self.say(format!("Couldn't save the layout: {e}"));
        }
    }

    fn play(&mut self) {
        self.paused = false;
        self.send(Command::Play { pairs: self.layout.pairs() });
    }

    fn layout_changed(&mut self) {
        self.config.set_layout(&self.layout);
        self.save();
        if self.mode != Mode::Stopped {
            self.send(Command::Play { pairs: self.layout.pairs() });
        }
    }

    fn watch(&mut self, keyboard: &Keyboard) {
        self.config.keyboard_id = Some(keyboard.id.clone());
        self.config.keyboard_name = Some(keyboard.name.clone());
        self.save();
        self.paused = false;
        self.send(Command::Watch { keyboard: keyboard.id.clone() });
    }

    fn on_event(&mut self, event: Event) {
        match event {
            Event::Checks { keyd_running, virtual_keyboard_error } => {
                self.access = Access::Granted;
                self.checks = Some(Checks { keyd_running, virtual_keyboard_error });
            }
            Event::Keyboards { list } => {
                self.keyboards = list;
                if self.watching.is_none() {
                    // Reconnect to the saved keyboard: same port first, then same name.
                    let saved = self
                        .keyboards
                        .iter()
                        .find(|k| self.config.keyboard_id.as_ref() == Some(&k.id))
                        .or_else(|| self.keyboards.iter().find(|k| self.config.keyboard_name.as_ref() == Some(&k.name)))
                        .filter(|k| k.blocked.is_none())
                        .cloned();
                    if let Some(keyboard) = saved {
                        self.watch(&keyboard);
                    }
                }
            }
            Event::Watching { keyboard } => {
                let started = keyboard.is_some();
                if !started {
                    self.rebinding = None;
                    self.held.clear();
                }
                self.watching = keyboard;
                if started && !self.paused && self.mode != Mode::Game {
                    self.play();
                }
            }
            Event::Key { code, down } => self.on_key(K::new(code), down),
            Event::Mode { mode } => self.mode = mode,
            Event::Error { message } => self.say(message),
        }
    }

    fn on_key(&mut self, key: K, down: bool) {
        if !down {
            self.held.remove(&key);
            return;
        }
        self.held.insert(key);
        self.last_press = Some((key, self.ctx.input(|i| i.time)));
        self.ctx.request_repaint_after(Duration::from_secs_f64(MIN_LIT));
        if let Some(action) = self.rebinding.take() {
            if self.layout.bind(action, key) {
                self.layout_changed();
                self.say(format!("{} now does {}.", key_name(key), ACTIONS[action].name));
            } else {
                self.say(format!("{} is kept for switching between the game and the desktop.", key_name(key)));
            }
        } else if key != TOGGLE && self.layout.action_of(key).is_none() {
            self.say(format!("{} isn't set to anything.", key_name(key)));
        }
    }

    /// Lit while held, and for a moment after a quick tap. Instant, never animated: it answers the keyboard.
    fn lit(&self, key: K) -> bool {
        let now = self.ctx.input(|i| i.time);
        self.held.contains(&key) || self.last_press.is_some_and(|(k, at)| k == key && now - at < MIN_LIT)
    }

    fn keyboard_name(&self) -> Option<String> {
        let id = self.watching.as_ref()?;
        self.keyboards.iter().find(|k| &k.id == id).map(|k| k.name.clone())
    }

    /// The single card: what is happening, the one thing to do about it, and who is who.
    fn status(&mut self, ui: &mut egui::Ui) {
        let keyd = self.checks.as_ref().is_some_and(|c| c.keyd_running);
        let virtual_keyboard_error = self.checks.as_ref().and_then(|c| c.virtual_keyboard_error.clone());
        let (color, title, detail, action): (Color32, &str, String, Option<(&str, Act, Color32)>) = match &self.access {
            Access::Asking => (MUTED, "Waiting for your password", "Reading keyboards needs admin rights.".into(), None),
            Access::Denied(why) => {
                (AMBER, "No keyboard access", format!("{why} Reading keyboards needs your password."), Some(("Try again", Act::Retry, P2)))
            }
            Access::Granted if keyd => (
                AMBER,
                "keyd is holding the keyboards",
                "It merges every keyboard into one. It starts again when you close couch-strive.".into(),
                Some(("Pause keyd", Act::PauseKeyd, P2)),
            ),
            Access::Granted if virtual_keyboard_error.is_some() => {
                (AMBER, "Can't type for Player 2", virtual_keyboard_error.unwrap_or_default(), None)
            }
            Access::Granted if self.watching.is_none() => {
                (MUTED, "Pick Player 2's keyboard", "Play starts as soon as you choose it.".into(), None)
            }
            Access::Granted => match self.mode {
                Mode::Game => (P2, "Playing", "F12 on Player 2's keyboard hands it back to the desktop.".into(), Some(("Pause", Act::Pause, CAP))),
                Mode::Desktop => (AMBER, "Paused", "Player 2's keyboard types normally. F12 on it resumes.".into(), Some(("Resume", Act::Resume, P2))),
                Mode::Stopped => (AMBER, "Paused", "Player 2's keyboard types normally.".into(), Some(("Resume", Act::Resume, P2))),
            },
        };

        let (mut clicked, mut picked, mut refresh, mut undo) = (None, None, false, false);
        egui::Frame::new()
            .fill(PANEL)
            .stroke(Stroke::new(1.0, LINE))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin::symmetric(14, 12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let (dot, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                    ui.painter().circle_filled(dot.center(), 5.0, color);
                    ui.label(RichText::new(title).size(18.0).strong().color(color));
                    if let Some((label, act, fill)) = action {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let button = egui::Button::new(RichText::new(label).strong().color(Color32::WHITE))
                                .fill(fill)
                                .min_size(egui::vec2(92.0, 28.0));
                            if ui.add(button).clicked() {
                                clicked = Some(act);
                            }
                        });
                    }
                });
                ui.label(RichText::new(detail).small().color(MUTED));
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    badge(ui, "1P", P1);
                    ui.label(RichText::new("your keyboard").small().color(MUTED));
                    ui.add_space(12.0);
                    badge(ui, "2P", P2);
                    ui.add_enabled_ui(matches!(self.access, Access::Granted), |ui| {
                        let current = self.keyboard_name().unwrap_or_else(|| "Choose a keyboard".to_owned());
                        let combo = egui::ComboBox::from_id_salt("keyboard")
                            .selected_text(current)
                            .width(ui.available_width().min(280.0))
                            .show_ui(ui, |ui| {
                                if self.keyboards.is_empty() {
                                    ui.label(RichText::new("No keyboards found").color(MUTED));
                                }
                                for keyboard in &self.keyboards {
                                    let selected = self.watching.as_ref() == Some(&keyboard.id);
                                    let label = match &keyboard.blocked {
                                        Some(why) => format!("{} ({why})", keyboard.name),
                                        None => keyboard.name.clone(),
                                    };
                                    let row = ui.add_enabled_ui(keyboard.blocked.is_none(), |ui| ui.selectable_label(selected, label));
                                    if row.inner.clicked() {
                                        picked = Some(keyboard.clone());
                                    }
                                }
                            });
                        // opening the list rescans, so a just-plugged keyboard shows up
                        refresh = combo.response.on_hover_text("Player 2's keyboard. Only this one is captured.").clicked();
                    });
                });
            });
        if !self.note.is_empty() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&self.note).small().color(MUTED));
                if self.removed.is_some() {
                    undo = ui.small_button("Undo").clicked();
                }
            });
        }

        match clicked {
            Some(Act::Retry) => self.start_helper(),
            Some(Act::PauseKeyd) => self.send(Command::PauseKeyd),
            Some(Act::Pause) => {
                self.paused = true;
                self.send(Command::Stop);
            }
            Some(Act::Resume) => self.play(),
            None => {}
        }
        if let Some(keyboard) = picked {
            self.watch(&keyboard);
        }
        if refresh {
            self.send(Command::List);
        }
        if undo && let Some((action, key)) = self.removed {
            self.layout.bind(action, key);
            self.layout_changed();
            self.say(format!("{} is back on {}.", key_name(key), ACTIONS[action].name));
        }
    }

    fn controls(&mut self, ui: &mut egui::Ui) {
        let can_add = self.watching.is_some();
        let (mut add, mut remove, mut cancel, mut sends, mut reset) = (None, None, false, None, false);
        egui::Grid::new("controls").num_columns(3).spacing([12.0, 4.0]).min_row_height(0.0).show(ui, |ui| {
            reset = ui.small_button("Reset").on_hover_text("Go back to the default layout").clicked();
            ui.label(RichText::new("Player 2 presses").small().color(MUTED));
            ui.label(RichText::new("Game gets").small().color(MUTED));
            ui.end_row();
            for (i, action) in ACTIONS.iter().enumerate() {
                if i > 0 && action.group != ACTIONS[i - 1].group {
                    // a small gap says "new group" without a label
                    ui.allocate_space(egui::vec2(0.0, 2.0));
                    ui.end_row();
                }
                let binding = &self.layout.0[i];
                let row_lit = binding.keys.iter().any(|&k| self.lit(k));
                ui.horizontal(|ui| {
                    ui.set_min_height(26.0);
                    ui.label(RichText::new(action.name).color(if row_lit { P2 } else { TEXT }));
                });
                ui.horizontal(|ui| {
                    for &key in &binding.keys {
                        let cap = keycap(ui, &cap_name(key), self.lit(key));
                        if cap.on_hover_text(format!("{}. Click to remove.", key_name(key))).clicked() {
                            remove = Some((i, key));
                        }
                    }
                    if self.rebinding == Some(i) {
                        let ghost = ghost_cap(ui, "press a key", true);
                        cancel = ghost.on_hover_text("Press the new key on Player 2's keyboard. Click to cancel.").clicked();
                    } else if can_add {
                        let ghost = ghost_cap(ui, "+", false);
                        if ghost.on_hover_text("Add a key: click, then press it on Player 2's keyboard.").clicked() {
                            add = Some(i);
                        }
                    }
                });
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt(("sends", i))
                        .selected_text(RichText::new(key_name(binding.sends)).monospace())
                        .width(118.0)
                        .height(320.0)
                        .show_ui(ui, |ui| {
                            for (name, keys) in OUTPUT_GROUPS {
                                ui.label(RichText::new(name).small().color(MUTED));
                                for &key in keys {
                                    if ui.selectable_label(key == binding.sends, key_name(key)).clicked() {
                                        sends = Some((i, key));
                                    }
                                }
                            }
                        })
                        .response
                        .on_hover_text("The key GGST's Controller 2 uses for this. Change it in GGST too.");
                    let clash = if shared_with_player_1(binding.sends) {
                        Some(format!("Player 1's keyboard can press {} too, and the game can't tell who did.", key_name(binding.sends)))
                    } else {
                        self.layout
                            .sharing_sends(i)
                            .map(|other| format!("{} sends this key too, so the game can't tell them apart.", ACTIONS[other].name))
                    };
                    if let Some(why) = clash {
                        ui.label(RichText::new("⚠").color(AMBER)).on_hover_text(why);
                    }
                });
                ui.end_row();
            }
        });

        if let Some((action, key)) = remove {
            self.layout.unbind(action, key);
            self.layout_changed();
            self.say(format!("Removed {} from {}.", key_name(key), ACTIONS[action].name));
            self.removed = Some((action, key));
        }
        if let Some(action) = add {
            self.rebinding = Some(action);
            self.say(format!("Press the new {} key on Player 2's keyboard.", ACTIONS[action].name));
        }
        if cancel {
            self.rebinding = None;
            self.say("");
        }
        if let Some((action, key)) = sends
            && self.layout.set_sends(action, key)
        {
            self.layout_changed();
        }
        if reset {
            self.layout = Layout::default();
            self.layout_changed();
            self.say("Back to the default layout.");
        }
    }

    fn ggst(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new(RichText::new("First time in GGST?").small().color(MUTED))
            .default_open(self.first_run)
            .show(ui, |ui| {
                ui.label(RichText::new("Settings > Keyboard Control Type: pick the two-player option.").small());
                ui.label(RichText::new("Button Settings, press 0 for Controller 2, pick Dash, then press Player 2's Dash key.").small());
            });
    }
}

/// Keycap legend, as short as the printed key.
fn cap_name(key: K) -> String {
    let short = match key {
        K::KEY_UP => "↑",
        K::KEY_DOWN => "↓",
        K::KEY_LEFT => "←",
        K::KEY_RIGHT => "→",
        K::KEY_LEFTSHIFT => "LShift",
        K::KEY_RIGHTSHIFT => "RShift",
        K::KEY_LEFTCTRL => "LCtrl",
        K::KEY_RIGHTCTRL => "RCtrl",
        K::KEY_LEFTALT => "LAlt",
        K::KEY_RIGHTALT => "RAlt",
        K::KEY_BACKSPACE => "Bksp",
        K::KEY_PAGEUP => "PgUp",
        K::KEY_PAGEDOWN => "PgDn",
        K::KEY_CAPSLOCK => "Caps",
        _ => return key_name(key),
    };
    short.to_owned()
}

fn badge(ui: &mut egui::Ui, text: &str, color: Color32) {
    egui::Frame::new()
        .fill(color)
        .corner_radius(CornerRadius::same(4))
        .inner_margin(Margin::symmetric(5, 1))
        .show(ui, |ui| ui.label(RichText::new(text).small().strong().color(Color32::WHITE)));
}

/// A key cap on its darker edge. A held key sinks onto the edge and turns Player 2 blue, like a key going down.
fn keycap(ui: &mut egui::Ui, text: &str, held: bool) -> egui::Response {
    const DEPTH: f32 = 2.0;
    let galley = ui.painter().layout_no_wrap(text.to_owned(), TextStyle::Monospace.resolve(ui.style()), TEXT);
    let size = egui::vec2((galley.size().x + 14.0).max(24.0), 22.0);
    let (rect, response) = ui.allocate_exact_size(size + egui::vec2(0.0, DEPTH), egui::Sense::click());
    let edge = egui::Rect::from_min_size(rect.min + egui::vec2(0.0, DEPTH), size);
    let cap = if held { edge } else { egui::Rect::from_min_size(rect.min, size) };
    let fill = if held {
        P2
    } else if response.hovered() {
        CAP_HOVER
    } else {
        CAP
    };
    let painter = ui.painter();
    painter.rect_filled(edge, CornerRadius::same(6), CAP_EDGE);
    painter.rect_filled(cap, CornerRadius::same(6), fill);
    painter.galley(cap.center() - galley.size() / 2.0, galley, TEXT);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, text));
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// An outlined, empty cap: "+" to add a key, or "press a key" while listening for one.
fn ghost_cap(ui: &mut egui::Ui, text: &str, listening: bool) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), TextStyle::Small.resolve(ui.style()), if listening { P2 } else { MUTED });
    let size = egui::vec2((galley.size().x + 12.0).max(24.0), 22.0);
    let (rect, response) = ui.allocate_exact_size(size + egui::vec2(0.0, 2.0), egui::Sense::click());
    let cap = egui::Rect::from_min_size(rect.min, size);
    let stroke = if listening || response.hovered() { P2 } else { LINE };
    ui.painter().rect_stroke(cap, CornerRadius::same(6), Stroke::new(1.0, stroke), StrokeKind::Inside);
    ui.painter().galley(cap.center() - galley.size() / 2.0, galley, MUTED);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, text));
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

impl eframe::App for App {
    fn logic(&mut self, _ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let Some(helper) = &mut self.helper else { return };
        let mut events = Vec::new();
        let gone = loop {
            match helper.events.try_recv() {
                Ok(event) => events.push(event),
                Err(mpsc::TryRecvError::Empty) => break false,
                Err(mpsc::TryRecvError::Disconnected) => break true,
            }
        };
        if gone {
            // pkexec exits with 126 or 127 when the password prompt is dismissed or fails
            let code = helper.child.wait().ok().and_then(|s| s.code());
            self.access = Access::Denied(match code {
                Some(126 | 127) => "The password prompt was closed.".to_owned(),
                _ => "The helper stopped.".to_owned(),
            });
            self.helper = None;
            self.checks = None;
            self.watching = None;
            self.rebinding = None;
            self.held.clear();
            self.mode = Mode::Stopped;
        }
        for event in events {
            self.on_event(event);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("status")
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(INK).inner_margin(Margin { left: 14, right: 14, top: 14, bottom: 6 }))
            .show(ui, |ui| self.status(ui));
        egui::CentralPanel::default().frame(egui::Frame::new().fill(INK).inner_margin(Margin::symmetric(14, 6))).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                self.controls(ui);
                ui.add_space(6.0);
                self.ggst(ui);
            });
        });
    }
}

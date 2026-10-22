//! The window. It runs as the user, keeps the config, and drives the root helper.

use crate::layout::{key_name, Config, Layout, ACTIONS, TOGGLE};
use crate::protocol::{self, Command, Event, Keyboard, Mode};
use eframe::egui::{self, Color32, RichText};
use evdev::KeyCode as K;
use std::{
    collections::VecDeque,
    io::{self, BufRead, BufReader},
    process::{Child, ChildStdin, Command as Process, Stdio},
    sync::mpsc,
    thread,
};

const GOOD: Color32 = Color32::from_rgb(90, 190, 110);
const WARN: Color32 = Color32::from_rgb(230, 170, 60);

pub fn run() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("couch-strive")
            .with_app_id("couch-strive")
            .with_inner_size([600.0, 780.0])
            .with_min_inner_size([480.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native("couch-strive", options, Box::new(|cc| Ok(Box::new(App::new(cc.egui_ctx.clone())))))
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

struct Checks {
    keyd_running: bool,
    virtual_keyboard_error: Option<String>,
}

struct App {
    ctx: egui::Context,
    config: Config,
    layout: Layout,
    helper: Option<Helper>,
    /// Why the helper is not running, shown next to the Grant access button.
    helper_note: Option<String>,
    checks: Option<Checks>,
    keyboards: Vec<Keyboard>,
    watching: Option<String>,
    mode: Mode,
    /// The action waiting for Player 2 to press its new key.
    rebinding: Option<usize>,
    tester: VecDeque<String>,
    status: String,
}

impl App {
    fn new(ctx: egui::Context) -> Self {
        let (config, status) = match Config::load() {
            Ok(config) => (config, String::new()),
            Err(e) => (Config::default(), format!("Could not read the saved layout, using defaults: {e}")),
        };
        App {
            ctx,
            layout: config.layout(),
            config,
            helper: None,
            helper_note: None,
            checks: None,
            keyboards: Vec::new(),
            watching: None,
            mode: Mode::Stopped,
            rebinding: None,
            tester: VecDeque::new(),
            status,
        }
    }

    fn start_helper(&mut self) {
        match Helper::start(self.ctx.clone()) {
            Ok(helper) => {
                self.helper = Some(helper);
                self.helper_note = None;
            }
            Err(e) => self.helper_note = Some(format!("Could not start the helper: {e}")),
        }
    }

    fn send(&mut self, cmd: Command) {
        if let Some(helper) = &mut self.helper
            && let Err(e) = protocol::send(&mut helper.stdin, &cmd)
        {
            self.status = format!("Could not reach the helper: {e}");
        }
    }

    fn save(&mut self) {
        if let Err(e) = self.config.save() {
            self.status = format!("Could not save: {e}");
        }
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
        self.tester.clear();
        self.send(Command::Watch { keyboard: keyboard.id.clone() });
    }

    fn on_event(&mut self, event: Event) {
        match event {
            Event::Checks { keyd_running, virtual_keyboard_error } => {
                self.checks = Some(Checks { keyd_running, virtual_keyboard_error })
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
                if keyboard.is_none() {
                    self.rebinding = None;
                }
                self.watching = keyboard;
            }
            Event::Key { code, down: true } => self.on_key(K::new(code)),
            Event::Key { down: false, .. } => {}
            Event::Mode { mode } => self.mode = mode,
            Event::Error { message } => self.status = message,
        }
    }

    fn on_key(&mut self, key: K) {
        if let Some(action) = self.rebinding.take() {
            if self.layout.bind(action, key) {
                self.status = format!("{} now does {}", key_name(key), ACTIONS[action].name);
                self.layout_changed();
            } else {
                self.status = format!("{} is kept for switching between the game and the desktop", key_name(key));
            }
        }
        let line = match (key, self.layout.action_of(key)) {
            (TOGGLE, _) => format!("{:<12} switches between game and desktop", key_name(key)),
            (_, Some(a)) => format!("{:<12} -> {:<12} {}", key_name(key), key_name(ACTIONS[a].sends), ACTIONS[a].name),
            (_, None) => format!("{:<12} not used", key_name(key)),
        };
        self.tester.push_front(line);
        self.tester.truncate(8);
    }

    fn setup(&mut self, ui: &mut egui::Ui) {
        ui.heading("Setup");
        let mut grant = false;
        let mut pause_keyd = false;
        egui::Grid::new("setup").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            mark(ui, Some(self.helper.is_some()));
            ui.horizontal_wrapped(|ui| {
                ui.label("Keyboard access");
                if self.helper.is_none() {
                    grant = ui.button("Grant access").on_hover_text("Asks for your password: reading keyboards needs admin rights").clicked();
                    if let Some(note) = &self.helper_note {
                        ui.colored_label(WARN, note);
                    }
                }
            });
            ui.end_row();

            mark(ui, self.checks.as_ref().map(|c| !c.keyd_running));
            ui.horizontal_wrapped(|ui| {
                ui.label("No key remapper holding the keyboards");
                if self.checks.as_ref().is_some_and(|c| c.keyd_running) {
                    pause_keyd = ui
                        .button("Pause keyd while playing")
                        .on_hover_text("keyd grabs every keyboard. It starts again when you close couch-strive.")
                        .clicked();
                }
            });
            ui.end_row();

            let virtual_keyboard = self.checks.as_ref().map(|c| c.virtual_keyboard_error.clone());
            mark(ui, virtual_keyboard.as_ref().map(Option::is_none));
            ui.horizontal_wrapped(|ui| {
                ui.label("Virtual keyboard for Player 2");
                if let Some(Some(error)) = virtual_keyboard {
                    ui.colored_label(WARN, error);
                }
            });
            ui.end_row();

            mark(ui, None);
            ui.label("In GGST: Settings > Keyboard Control Type > the two-player option");
            ui.end_row();

            mark(ui, None);
            ui.label("In GGST: Button Settings, press 0, pick Dash, press Player 2's Dash key");
            ui.end_row();
        });
        if grant {
            self.start_helper();
        }
        if pause_keyd {
            self.send(Command::PauseKeyd);
        }
    }

    fn keyboard_picker(&mut self, ui: &mut egui::Ui) {
        ui.heading("Player 2's keyboard");
        let current = self
            .watching
            .as_ref()
            .and_then(|id| self.keyboards.iter().find(|k| &k.id == id))
            .map_or("Choose a keyboard", |k| k.name.as_str())
            .to_owned();
        let mut picked = None;
        let mut refresh = false;
        ui.add_enabled_ui(self.helper.is_some() && self.mode != Mode::Game, |ui| {
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("keyboard").selected_text(current).width(320.0).show_ui(ui, |ui| {
                    for keyboard in &self.keyboards {
                        let selected = self.watching.as_ref() == Some(&keyboard.id);
                        let label = match &keyboard.blocked {
                            Some(why) => format!("{} ({why})", keyboard.name),
                            None => keyboard.name.clone(),
                        };
                        let usable = keyboard.blocked.is_none();
                        if ui.add_enabled_ui(usable, |ui| ui.selectable_label(selected, label)).inner.clicked() {
                            picked = Some(keyboard.clone());
                        }
                    }
                });
                refresh = ui.button("Refresh").clicked();
            });
        });
        ui.weak("Only this keyboard is captured. Player 1's keyboard keeps working as normal.");
        if let Some(keyboard) = picked {
            self.watch(&keyboard);
        }
        if refresh {
            self.send(Command::List);
        }
    }

    fn layout_editor(&mut self, ui: &mut egui::Ui) {
        ui.heading("Player 2's layout");
        ui.weak("Click + and press a key on Player 2's keyboard. Click a key to remove it.");
        let can_rebind = self.watching.is_some();
        let (mut add, mut remove, mut cancel) = (None, None, false);
        egui::Grid::new("layout").num_columns(3).striped(true).spacing([16.0, 6.0]).show(ui, |ui| {
            ui.strong("Action");
            ui.strong("Player 2 presses");
            ui.strong("Game gets");
            ui.end_row();
            for (i, action) in ACTIONS.iter().enumerate() {
                ui.label(action.name);
                ui.horizontal(|ui| {
                    for &key in &self.layout.0[i] {
                        if ui.small_button(key_name(key)).on_hover_text("Click to remove").clicked() {
                            remove = Some((i, key));
                        }
                    }
                    if self.rebinding == Some(i) {
                        ui.colored_label(WARN, "press a key...");
                        cancel = ui.small_button("Cancel").clicked();
                    } else if ui
                        .add_enabled(can_rebind, egui::Button::new("+").small())
                        .on_hover_text("Add a key")
                        .on_disabled_hover_text("Pick Player 2's keyboard first")
                        .clicked()
                    {
                        add = Some(i);
                    }
                });
                ui.label(key_name(action.sends));
                ui.end_row();
            }
        });
        if let Some((action, key)) = remove {
            self.layout.unbind(action, key);
            self.layout_changed();
        }
        if let Some(action) = add {
            self.rebinding = Some(action);
            self.status = format!("Press the new {} key on Player 2's keyboard", ACTIONS[action].name);
        }
        if cancel {
            self.rebinding = None;
            self.status.clear();
        }
        if ui.button("Reset to defaults").clicked() {
            self.layout = Layout::default();
            self.layout_changed();
        }
    }

    fn tester(&self, ui: &mut egui::Ui) {
        ui.heading("Key tester");
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            if self.tester.is_empty() {
                ui.weak(if self.watching.is_some() {
                    "Press keys on Player 2's keyboard to see what the game gets."
                } else {
                    "Pick Player 2's keyboard to test it."
                });
            }
            for line in &self.tester {
                ui.monospace(line);
            }
        });
    }

    fn controls(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        let ready = self.watching.is_some() && self.checks.as_ref().is_some_and(|c| c.virtual_keyboard_error.is_none());
        let (badge, color, text) = match self.mode {
            Mode::Game => ("PLAYING", GOOD, "Player 2's keyboard drives GGST's Controller 2. F12 on it switches to the desktop."),
            Mode::Desktop => ("DESKTOP", WARN, "Player 2's keyboard types normally. F12 on it goes back to the game."),
            // Picking a keyboard only watches it; nothing reaches the game until Start.
            Mode::Stopped if ready => ("NOT PLAYING", WARN, "Click Start to send Player 2's keys to the game."),
            Mode::Stopped => ("NOT PLAYING", Color32::GRAY, "Grant access and pick Player 2's keyboard, then click Start."),
        };
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(badge).strong().color(color));
            ui.label(text);
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let big = |text: &str| egui::Button::new(RichText::new(text).size(18.0).strong()).min_size(egui::vec2(140.0, 36.0));
            if self.mode == Mode::Stopped {
                let start = big("Start").fill(Color32::from_rgb(40, 140, 70));
                if ui.add_enabled(ready, start).on_disabled_hover_text("Finish the setup and pick Player 2's keyboard").clicked() {
                    self.send(Command::Play { pairs: self.layout.pairs() });
                }
            } else if ui.add(big("Stop")).clicked() {
                self.send(Command::Stop);
            }
            ui.label(&self.status);
        });
        ui.add_space(8.0);
    }
}

fn mark(ui: &mut egui::Ui, ok: Option<bool>) {
    match ok {
        Some(true) => ui.colored_label(GOOD, "✔"),
        Some(false) => ui.colored_label(WARN, "⚠"),
        None => ui.weak("•"),
    };
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
            self.helper_note = Some(match code {
                Some(126 | 127) => "Access was not granted.".to_owned(),
                _ => "The helper stopped.".to_owned(),
            });
            self.helper = None;
            self.checks = None;
            self.watching = None;
            self.rebinding = None;
            self.mode = Mode::Stopped;
        }
        for event in events {
            self.on_event(event);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::bottom("controls").show(ui, |ui| self.controls(ui));
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                self.setup(ui);
                ui.add_space(14.0);
                self.keyboard_picker(ui);
                ui.add_space(14.0);
                self.layout_editor(ui);
                ui.add_space(14.0);
                self.tester(ui);
            });
        });
    }
}

#![windows_subsystem = "windows"]

mod settings;
mod theme;

use eframe::egui;
use settings::{AppSettings, CalibParam, SettingsLoadResult};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;
use tokio_modbus::prelude::*;
use tokio_modbus::client::{tcp, rtu, Context};
use tokio_modbus::Slave;
use tokio_serial::SerialStream;

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const C_STATE_ADDR: u16 = 190;
const TIMEOUT_SECS: u64 = 2; // Timeout for unresponsive devices
const POLL_INTERVAL_MS: u64 = 500; // How often to re-read the current-value register

// --- Data Structures ---
// CalibParam and its factory defaults now live in settings.rs (they're part
// of the persisted settings file, not a compiled-in constant).

#[derive(PartialEq)]
enum CalibStep {
    SelectParam,
    Step1SetZeroExternally,
    Step2WriteOffsetValue,
    Step3SetTargetExternally,
    Step4WriteGainValue,
    Finished,
}

// --- Communication Protocols ---

pub enum ModbusConfig {
    Tcp { ip: String, slave_id: Option<u8> },
    Rtu { port: String, baud: u32, slave_id: u8 },
}

pub enum ModbusCommand {
    Connect(ModbusConfig),
    WriteRegister(u16, u16),
    WriteRegisterDelayed(u16, u16, u64),
    ReadRegister(u16),
    Disconnect,
}

pub enum AppEvent {
    Connected,
    ConnectionError(String),
    WriteError(String),
    ReadError(String),
    RegisterValue(u16, u16), // (addr, value)
    Disconnected,
}

// --- Background Modbus Worker ---
// (unchanged from the original — protocol handling isn't a UI concern)

fn spawn_modbus_worker(event_tx: mpsc::Sender<AppEvent>) -> mpsc::Sender<ModbusCommand> {
    let (tx, rx) = mpsc::channel::<ModbusCommand>();
    let self_tx = tx.clone();

    thread::spawn(move || {
        let rt = Runtime::new().expect("Failed to create Tokio runtime");

        rt.block_on(async {
            let mut modbus_ctx: Option<Context> = None;

            while let Ok(cmd) = rx.recv() {
                match cmd {
                    ModbusCommand::Connect(config) => {
                        match config {
                            ModbusConfig::Tcp { ip, slave_id } => {
                                if let Ok(socket_addr) = ip.parse() {
                                    if let Some(id) = slave_id {
                                        match tokio::time::timeout(Duration::from_secs(TIMEOUT_SECS), tcp::connect_slave(socket_addr, Slave(id))).await {
                                            Ok(Ok(ctx)) => {
                                                modbus_ctx = Some(ctx);
                                                let _ = event_tx.send(AppEvent::Connected);
                                            }
                                            Ok(Err(e)) => { let _ = event_tx.send(AppEvent::ConnectionError(e.to_string())); }
                                            Err(_) => { let _ = event_tx.send(AppEvent::ConnectionError("Connection Timeout".into())); }
                                        }
                                    } else {
                                        match tokio::time::timeout(Duration::from_secs(TIMEOUT_SECS), tcp::connect(socket_addr)).await {
                                            Ok(Ok(ctx)) => {
                                                modbus_ctx = Some(ctx);
                                                let _ = event_tx.send(AppEvent::Connected);
                                            }
                                            Ok(Err(e)) => { let _ = event_tx.send(AppEvent::ConnectionError(e.to_string())); }
                                            Err(_) => { let _ = event_tx.send(AppEvent::ConnectionError("Connection Timeout".into())); }
                                        }
                                    }
                                } else {
                                    let _ = event_tx.send(AppEvent::ConnectionError("Invalid IP format".into()));
                                }
                            }
                            ModbusConfig::Rtu { port, baud, slave_id } => {
                                let builder = tokio_serial::new(port, baud);
                                match SerialStream::open(&builder) {
                                    Ok(serial_stream) => {
                                        let ctx = rtu::attach_slave(serial_stream, Slave(slave_id));
                                        modbus_ctx = Some(ctx);
                                        let _ = event_tx.send(AppEvent::Connected);
                                    }
                                    Err(e) => {
                                        let _ = event_tx.send(AppEvent::ConnectionError(e.to_string()));
                                    }
                                }
                            }
                        }
                    }
                    ModbusCommand::WriteRegister(addr, val) => {
                        if let Some(ctx) = &mut modbus_ctx {
                            match tokio::time::timeout(
                                Duration::from_secs(TIMEOUT_SECS),
                                ctx.write_multiple_registers(addr, &[val])
                            ).await {
                                Ok(Ok(_)) => { /* Success */ }
                                Ok(Err(e)) => { let _ = event_tx.send(AppEvent::WriteError(e.to_string())); }
                                Err(_) => {
                                    let _ = event_tx.send(AppEvent::WriteError("Timeout: Device ignored request. Check Device ID.".into()));
                                }
                            }
                        } else {
                            let _ = event_tx.send(AppEvent::WriteError("Modbus is not connected.".into()));
                        }
                    }
                    ModbusCommand::WriteRegisterDelayed(addr, val, delay_secs) => {
                        let delayed_tx = self_tx.clone();
                        thread::spawn(move || {
                            std::thread::sleep(Duration::from_secs(delay_secs));
                            let _ = delayed_tx.send(ModbusCommand::WriteRegister(addr, val));
                        });
                    }
                    ModbusCommand::ReadRegister(addr) => {
                        if addr == 0 {
                            continue;
                        }
                        if let Some(ctx) = &mut modbus_ctx {
                            match tokio::time::timeout(
                                Duration::from_secs(TIMEOUT_SECS),
                                ctx.read_holding_registers(addr, 1)
                            ).await {
                                Ok(Ok(Ok(values))) => {
                                    if let Some(&val) = values.first() {
                                        let _ = event_tx.send(AppEvent::RegisterValue(addr, val));
                                    }
                                }
                                Ok(Ok(Err(exception))) => {
                                    let _ = event_tx.send(AppEvent::ReadError(format!("Modbus exception: {:?}", exception)));
                                }
                                Ok(Err(e)) => { let _ = event_tx.send(AppEvent::ReadError(e.to_string())); }
                                Err(_) => { let _ = event_tx.send(AppEvent::ReadError("Read Timeout".into())); }
                            }
                        }
                    }
                    ModbusCommand::Disconnect => {
                        if let Some(mut ctx) = modbus_ctx.take() {
                            let _ = tokio::time::timeout(Duration::from_secs(1), ctx.disconnect()).await;
                            let _ = event_tx.send(AppEvent::Disconnected);
                        }
                    }
                }
            }
        });
    });

    tx
}

fn fetch_available_ports() -> Vec<String> {
    tokio_serial::available_ports()
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.port_name)
        .collect()
}

fn load_icon() -> egui::IconData {
    // Embedded at compile time — no external asset path to break on deployment.
    let bytes = include_bytes!("../assets/icon.png");
    let image = image::load_from_memory(bytes)
        .expect("bundled assets/icon.png is invalid")
        .into_rgba8();
    let (width, height) = image.dimensions();
    egui::IconData {
        rgba: image.into_raw(),
        width,
        height,
    }
}

// --- egui Application ---

struct CalibratorApp {
    modbus_tx: mpsc::Sender<ModbusCommand>,
    app_rx: mpsc::Receiver<AppEvent>,

    is_connected: bool,
    is_connecting: bool,
    error_message: Option<String>,
    status_message: Option<String>,

    is_tcp: bool,
    tcp_ip: String,
    enforce_tcp_device_id: bool,

    rtu_port: String,
    available_ports: Vec<String>,
    rtu_baud: String,
    device_id: String,

    parameters: Vec<CalibParam>,
    selected_idx: usize,
    current_step: CalibStep,
    input_offset_val: String,
    input_gain_val: String,

    scale_factor: String,
    auto_reset_cstate: bool,
    auto_reset_delay: String,

    polling_addr: Option<u16>,
    current_val_raw: Option<u16>,
    last_poll: Instant,

    // --- Settings / menu / chrome state ---
    dark_mode: bool,
    show_about: bool,
    show_param_editor: bool,
    settings_path_display: String,
}

impl CalibratorApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let (app_tx, app_rx) = mpsc::channel();
        let modbus_tx = spawn_modbus_worker(app_tx);

        let (settings, load_result) = AppSettings::load_or_create();
        let status_message = match load_result {
            SettingsLoadResult::Loaded => None,
            SettingsLoadResult::CreatedDefault => Some(format!(
                "No settings file found — created one with defaults at {}",
                AppSettings::settings_file_path().display()
            )),
            SettingsLoadResult::CorruptFellBackToDefault(err) => Some(format!(
                "Settings file was unreadable ({err}) — using defaults. The old file was left untouched."
            )),
        };

        let mut available_ports = fetch_available_ports();
        if available_ports.is_empty() {
            available_ports.push(settings.connection.rtu_port.clone());
        }

        Self {
            modbus_tx,
            app_rx,
            is_connected: false,
            is_connecting: false,
            error_message: None,
            status_message,
            is_tcp: settings.connection.is_tcp,
            tcp_ip: settings.connection.tcp_ip,
            enforce_tcp_device_id: settings.connection.enforce_tcp_device_id,
            rtu_port: settings.connection.rtu_port,
            available_ports,
            rtu_baud: settings.connection.rtu_baud,
            device_id: settings.connection.device_id,
            parameters: settings.parameters,
            selected_idx: 0,
            current_step: CalibStep::SelectParam,
            input_offset_val: "0".to_string(),
            input_gain_val: "".to_string(),
            scale_factor: settings.scale_factor,
            auto_reset_cstate: settings.auto_reset_cstate,
            auto_reset_delay: settings.auto_reset_delay,
            polling_addr: None,
            current_val_raw: None,
            last_poll: Instant::now() - Duration::from_secs(1),
            dark_mode: settings.dark_mode,
            show_about: false,
            show_param_editor: false,
            settings_path_display: AppSettings::settings_file_path().display().to_string(),
        }
    }

    fn current_settings(&self) -> AppSettings {
        AppSettings {
            connection: settings::ConnectionSettings {
                is_tcp: self.is_tcp,
                tcp_ip: self.tcp_ip.clone(),
                enforce_tcp_device_id: self.enforce_tcp_device_id,
                rtu_port: self.rtu_port.clone(),
                rtu_baud: self.rtu_baud.clone(),
                device_id: self.device_id.clone(),
            },
            scale_factor: self.scale_factor.clone(),
            auto_reset_cstate: self.auto_reset_cstate,
            auto_reset_delay: self.auto_reset_delay.clone(),
            dark_mode: self.dark_mode,
            parameters: self.parameters.clone(),
            schema_version: 1,
        }
    }

    fn apply_settings(&mut self, settings: AppSettings) {
        self.is_tcp = settings.connection.is_tcp;
        self.tcp_ip = settings.connection.tcp_ip;
        self.enforce_tcp_device_id = settings.connection.enforce_tcp_device_id;
        self.rtu_port = settings.connection.rtu_port;
        self.rtu_baud = settings.connection.rtu_baud;
        self.device_id = settings.connection.device_id;
        self.scale_factor = settings.scale_factor;
        self.auto_reset_cstate = settings.auto_reset_cstate;
        self.auto_reset_delay = settings.auto_reset_delay;
        self.dark_mode = settings.dark_mode;
        self.parameters = settings.parameters;
        // The imported/reset map might have fewer rows than before, or the
        // previously selected row might no longer exist — clamp so we never
        // index out of bounds on the next frame.
        if self.selected_idx >= self.parameters.len() {
            self.selected_idx = 0;
        }
    }

    fn save_settings(&mut self) {
        match self.current_settings().save() {
            Ok(()) => self.status_message = Some("Settings saved.".to_string()),
            Err(e) => self.error_message = Some(format!("Could not save settings: {e}")),
        }
    }

    /// Renders the raw + scaled ("point") value for the currently selected parameter.
    fn show_current_value(&self, ui: &mut egui::Ui) {
        let param = &self.parameters[self.selected_idx];
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("Live {}", param.name)).strong());
                ui.separator();
                if param.current_val_addr == 0 {
                    ui.colored_label(theme::WARNING, "Register not mapped yet (TODO)");
                    return;
                }
                match self.current_val_raw {
                    Some(raw) => {
                        let scale: f64 = self.scale_factor.trim().parse().unwrap_or(10.0);
                        let scaled = raw as f64 / scale;
                        ui.monospace(format!("Raw: {}", raw));
                        ui.add_space(10.0);
                        ui.monospace(format!("Scaled: {:.2}", scaled));
                    }
                    None => {
                        ui.label("—");
                    }
                }
            });
        });
    }

    fn poll_current_value(&mut self) {
        if !self.is_connected {
            return;
        }
        let addr = self.parameters[self.selected_idx].current_val_addr;
        if addr == 0 {
            return;
        }
        if self.polling_addr != Some(addr) {
            self.polling_addr = Some(addr);
            self.current_val_raw = None;
            self.last_poll = Instant::now() - Duration::from_secs(1);
        }
        if self.last_poll.elapsed() >= Duration::from_millis(POLL_INTERVAL_MS) {
            self.last_poll = Instant::now();
            let _ = self.modbus_tx.send(ModbusCommand::ReadRegister(addr));
        }
    }

    fn menu_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("💾  Save Settings").clicked() {
                        self.save_settings();
                        ui.close_menu();
                    }
                    if ui.button("↺  Reload Settings from Disk").clicked() {
                        let (settings, result) = AppSettings::load_or_create();
                        self.apply_settings(settings);
                        self.status_message = Some(match result {
                            SettingsLoadResult::Loaded => "Settings reloaded.".to_string(),
                            SettingsLoadResult::CreatedDefault => "No file existed — created defaults.".to_string(),
                            SettingsLoadResult::CorruptFellBackToDefault(e) => format!("File corrupt ({e}); used defaults."),
                        });
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Export Settings As…").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .set_file_name("calibrator_settings.json")
                            .add_filter("JSON", &["json"])
                            .save_file()
                        {
                            match self.current_settings().export_to(&path) {
                                Ok(()) => self.status_message = Some(format!("Exported to {}", path.display())),
                                Err(e) => self.error_message = Some(format!("Export failed: {e}")),
                            }
                        }
                        ui.close_menu();
                    }
                    if ui.button("Import Settings…").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("JSON", &["json"])
                            .pick_file()
                        {
                            match AppSettings::import_from(&path) {
                                Ok(settings) => {
                                    self.apply_settings(settings);
                                    self.status_message = Some(format!("Imported from {}", path.display()));
                                }
                                Err(e) => self.error_message = Some(format!("Import failed: {e}")),
                            }
                        }
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Edit Register Map…").clicked() {
                        self.show_param_editor = true;
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Reset to Defaults").clicked() {
                        self.apply_settings(AppSettings::default());
                        self.status_message = Some("Reset to default settings (not yet saved).".to_string());
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Exit").clicked() {
                        std::process::exit(0);
                    }
                });

                ui.menu_button("View", |ui| {
                    if ui.checkbox(&mut self.dark_mode, "Dark Mode").changed() {
                        theme::apply(ctx, self.dark_mode);
                    }
                });

                ui.menu_button("Help", |ui| {
                    if ui.button("About").clicked() {
                        self.show_about = true;
                        ui.close_menu();
                    }
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (color, text) = if self.is_connected {
                        (theme::SUCCESS, "Connected")
                    } else if self.is_connecting {
                        (theme::WARNING, "Connecting…")
                    } else {
                        (theme::DANGER, "Disconnected")
                    };
                    theme::status_dot(ui, color, text);
                });
            });
        });
    }

    fn about_window(&mut self, ctx: &egui::Context) {
        if !self.show_about {
            return;
        }
        let mut open = self.show_about;
        egui::Window::new("About")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.heading("Modbus Master Auto-Calibrator");
                ui.label(format!("Version {APP_VERSION}"));
                ui.add_space(6.0);
                ui.label("A guided offset/gain calibration tool for Modbus TCP/RTU metering devices.");
                ui.add_space(6.0);
                ui.label(format!("Settings file: {}", self.settings_path_display));
            });
        self.show_about = open;
    }

    /// Editable view of the calibration register map. Edits here live in
    /// memory until "Save Settings" (or Connect, which auto-saves) writes
    /// them to disk — "Cancel/Close" just dismisses the window without
    /// discarding anything, since there's no separate draft copy; use
    /// File → Reload Settings from Disk to throw away unsaved edits.
    fn param_editor_window(&mut self, ctx: &egui::Context) {
        if !self.show_param_editor {
            return;
        }
        let mut open = self.show_param_editor;
        let mut add_row = false;
        let mut remove_idx: Option<usize> = None;

        egui::Window::new("Edit Register Map")
            .open(&mut open)
            .default_width(560.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.label(egui::RichText::new(
                    "Changes here are not written to disk until you Save Settings.",
                ).weak());
                ui.add_space(6.0);

                egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                    egui::Grid::new("param_grid")
                        .striped(true)
                        .num_columns(7)
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new("Name").strong());
                            ui.label(egui::RichText::new("Offset Addr").strong());
                            ui.label(egui::RichText::new("Gain Addr").strong());
                            ui.label(egui::RichText::new("C_State (Off.)").strong());
                            ui.label(egui::RichText::new("C_State (Gain)").strong());
                            ui.label(egui::RichText::new("Live Val Addr").strong());
                            ui.label("");
                            ui.end_row();

                            for (i, param) in self.parameters.iter_mut().enumerate() {
                                ui.add(egui::TextEdit::singleline(&mut param.name).desired_width(70.0));
                                ui.add(egui::DragValue::new(&mut param.offset_addr));
                                ui.add(egui::DragValue::new(&mut param.gain_addr));
                                ui.add(egui::DragValue::new(&mut param.cstate_offset_val));
                                ui.add(egui::DragValue::new(&mut param.cstate_gain_val));
                                ui.add(egui::DragValue::new(&mut param.current_val_addr))
                                    .on_hover_text("0 = not yet mapped");
                                if theme::danger_button(ui, "X").clicked() {
                                    remove_idx = Some(i);
                                }
                                ui.end_row();
                            }
                        });
                });

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("➕ Add Parameter").clicked() {
                        add_row = true;
                    }
                    if theme::primary_button(ui, "💾 Save Settings").clicked() {
                        self.save_settings();
                    }
                    if ui.button("Restore Factory Register Map").clicked() {
                        self.parameters = settings::default_parameters();
                        self.status_message = Some("Register map restored to factory defaults (not yet saved).".to_string());
                    }
                });
            });

        if add_row {
            self.parameters.push(settings::CalibParam {
                name: "NEW".to_string(),
                offset_addr: 0,
                gain_addr: 0,
                cstate_offset_val: 0,
                cstate_gain_val: 0,
                current_val_addr: 0,
            });
        }
        if let Some(idx) = remove_idx {
            self.parameters.remove(idx);
            if self.selected_idx >= self.parameters.len() {
                self.selected_idx = self.parameters.len().saturating_sub(1);
            }
        }

        self.show_param_editor = open;
    }
}

impl eframe::App for CalibratorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(event) = self.app_rx.try_recv() {
            match event {
                AppEvent::Connected => {
                    self.is_connected = true;
                    self.is_connecting = false;
                    self.error_message = None;
                    self.status_message = Some("Connected.".to_string());
                }
                AppEvent::ConnectionError(err) => {
                    self.is_connecting = false;
                    self.error_message = Some(format!("Connection Failed: {}", err));
                }
                AppEvent::WriteError(err) => {
                    self.error_message = Some(format!("Write Error: {}", err));
                }
                AppEvent::ReadError(err) => {
                    self.error_message = Some(format!("Read Error: {}", err));
                }
                AppEvent::RegisterValue(addr, val) => {
                    if self.polling_addr == Some(addr) {
                        self.current_val_raw = Some(val);
                    }
                }
                AppEvent::Disconnected => {
                    self.is_connected = false;
                    self.status_message = Some("Disconnected.".to_string());
                }
            }
        }

        self.poll_current_value();
        theme::apply(ctx, self.dark_mode);

        self.menu_bar(ctx);
        self.about_window(ctx);
        self.param_editor_window(ctx);

        ctx.request_repaint();

        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if let Some(msg) = &self.status_message {
                    ui.label(egui::RichText::new(msg).weak());
                } else {
                    ui.label(egui::RichText::new("Ready").weak());
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(4.0);
            ui.heading("Modbus Master Auto-Calibrator");
            ui.add_space(8.0);

            theme::card(ui).show(ui, |ui| {
                ui.label(egui::RichText::new("Global Settings").strong());
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label("Scale Factor:");
                    ui.text_edit_singleline(&mut self.scale_factor);
                });
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.auto_reset_cstate, "Auto-Reset C_State (Addr 190) after Calibration");
                    if self.auto_reset_cstate {
                        ui.label("Delay (sec):");
                        ui.add(egui::TextEdit::singleline(&mut self.auto_reset_delay).desired_width(40.0));
                    }
                });
            });
            ui.add_space(10.0);

            if let Some(err) = &self.error_message {
                egui::Frame::none()
                    .fill(theme::DANGER.gamma_multiply(0.15))
                    .rounding(egui::Rounding::same(6.0))
                    .inner_margin(egui::Margin::same(10.0))
                    .show(ui, |ui| {
                        ui.colored_label(theme::DANGER, format!("⚠  {err}"));
                    });
                ui.add_space(10.0);
            }

            if !self.is_connected {
                theme::card(ui).show(ui, |ui| {
                    ui.label(egui::RichText::new("Connection").strong());
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut self.is_tcp, true, "Modbus TCP");
                        ui.radio_value(&mut self.is_tcp, false, "Modbus RTU");
                    });
                    ui.add_space(8.0);

                    if self.is_tcp {
                        ui.horizontal(|ui| {
                            ui.label("IP Address:");
                            ui.text_edit_singleline(&mut self.tcp_ip);
                        });
                        ui.checkbox(&mut self.enforce_tcp_device_id, "Enforce Device ID (Unit ID)");
                        if self.enforce_tcp_device_id {
                            ui.horizontal(|ui| {
                                ui.label("Device ID:");
                                ui.text_edit_singleline(&mut self.device_id);
                            });
                        }
                    } else {
                        ui.horizontal(|ui| {
                            ui.label("COM Port:");
                            ui.add(egui::TextEdit::singleline(&mut self.rtu_port).desired_width(120.0));
                            egui::ComboBox::from_id_source("com_port_dropdown")
                                .selected_text("↓")
                                .width(10.0)
                                .show_ui(ui, |ui| {
                                    if self.available_ports.is_empty() {
                                        ui.label("No ports found.");
                                    } else {
                                        for port in &self.available_ports {
                                            ui.selectable_value(&mut self.rtu_port, port.clone(), port);
                                        }
                                    }
                                });
                            if ui.button("🔄 Refresh").clicked() {
                                self.available_ports = fetch_available_ports();
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("Baud Rate:");
                            ui.text_edit_singleline(&mut self.rtu_baud);
                        });
                        ui.horizontal(|ui| {
                            ui.label("Device ID (Slave ID):");
                            ui.text_edit_singleline(&mut self.device_id);
                        });
                    }

                    ui.add_space(14.0);

                    if self.is_connecting {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Connecting...");
                        });
                    } else if theme::primary_button(ui, "🔌  Connect").clicked() {
                        self.is_connecting = true;
                        self.error_message = None;

                        let parsed_id = self.device_id.parse().unwrap_or(1);
                        let config = if self.is_tcp {
                            let slave_id = if self.enforce_tcp_device_id { Some(parsed_id) } else { None };
                            ModbusConfig::Tcp { ip: self.tcp_ip.clone(), slave_id }
                        } else {
                            ModbusConfig::Rtu {
                                port: self.rtu_port.clone(),
                                baud: self.rtu_baud.parse().unwrap_or(9600),
                                slave_id: parsed_id,
                            }
                        };
                        let _ = self.modbus_tx.send(ModbusCommand::Connect(config));
                        // Persist working connection settings automatically so next
                        // launch starts from what actually worked before.
                        self.save_settings();
                    }
                });
            } else {
                let param = self.parameters[self.selected_idx].clone();

                theme::card(ui).show(ui, |ui| {
                    match self.current_step {
                        CalibStep::SelectParam => {
                            ui.label(egui::RichText::new("Select Parameter to Calibrate").strong());
                            ui.add_space(6.0);
                            egui::ComboBox::from_label("")
                                .selected_text(param.name)
                                .show_ui(ui, |ui| {
                                    for (i, p) in self.parameters.iter().enumerate() {
                                        ui.selectable_value(&mut self.selected_idx, i, p.name.as_str());
                                    }
                                });

                            ui.add_space(10.0);
                            self.show_current_value(ui);

                            ui.add_space(16.0);
                            if theme::primary_button(ui, "▶  Start Calibration").clicked() {
                                self.current_step = CalibStep::Step1SetZeroExternally;
                                self.error_message = None;
                            }
                        }

                        CalibStep::Step1SetZeroExternally => {
                            ui.label(egui::RichText::new(format!("Calibrating: {}", param.name)).heading());
                            ui.label("Step 1: Externally make the actual reading value of the parameter 0.");
                            ui.add_space(10.0);
                            self.show_current_value(ui);
                            ui.add_space(10.0);
                            if theme::primary_button(ui, "Done — Proceed to Step 2").clicked() {
                                self.current_step = CalibStep::Step2WriteOffsetValue;
                            }
                        }

                        CalibStep::Step2WriteOffsetValue => {
                            ui.label(egui::RichText::new(format!("Step 2: Enter measured value for {} Offset", param.name)).strong());
                            ui.horizontal(|ui| {
                                ui.label("Offset Value:");
                                ui.text_edit_singleline(&mut self.input_offset_val);
                            });
                            ui.add_space(10.0);
                            self.show_current_value(ui);

                            ui.add_space(10.0);
                            if theme::primary_button(ui, "Write Offset & Trigger C_State").clicked() {
                                self.error_message = None;

                                let input_f64: f64 = self.input_offset_val.trim().parse().unwrap_or(0.0);
                                let scale: f64 = self.scale_factor.trim().parse().unwrap_or(10.0);
                                let scaled_val: u16 = (input_f64 * scale).round() as u16;

                                if self.auto_reset_cstate {
                                    let _ = self.modbus_tx.send(ModbusCommand::WriteRegister(C_STATE_ADDR, 0));
                                }
                                let _ = self.modbus_tx.send(ModbusCommand::WriteRegister(param.offset_addr, scaled_val));
                                let _ = self.modbus_tx.send(ModbusCommand::WriteRegister(C_STATE_ADDR, param.cstate_offset_val));

                                self.current_step = CalibStep::Step3SetTargetExternally;
                            }
                        }

                        CalibStep::Step3SetTargetExternally => {
                            ui.label(egui::RichText::new("Step 3: Set the actual parameter to the desired value externally.").strong());
                            ui.add_space(10.0);
                            self.show_current_value(ui);
                            ui.add_space(10.0);
                            if theme::primary_button(ui, "Done — Proceed to Step 4").clicked() {
                                self.current_step = CalibStep::Step4WriteGainValue;
                            }
                        }

                        CalibStep::Step4WriteGainValue => {
                            ui.label(egui::RichText::new(format!("Step 4: Enter the desired measured value for {} Gain", param.name)).strong());
                            ui.horizontal(|ui| {
                                ui.label("Desired Value:");
                                ui.text_edit_singleline(&mut self.input_gain_val);
                            });
                            ui.add_space(10.0);
                            self.show_current_value(ui);

                            ui.add_space(10.0);
                            if theme::primary_button(ui, "Write Gain & Trigger C_State").clicked() {
                                self.error_message = None;

                                let input_f64: f64 = self.input_gain_val.trim().parse().unwrap_or(0.0);
                                let scale: f64 = self.scale_factor.trim().parse().unwrap_or(10.0);
                                let scaled_val: u16 = (input_f64 * scale).round() as u16;

                                if self.auto_reset_cstate {
                                    let _ = self.modbus_tx.send(ModbusCommand::WriteRegister(C_STATE_ADDR, 0));
                                }
                                let _ = self.modbus_tx.send(ModbusCommand::WriteRegister(param.gain_addr, scaled_val));
                                let _ = self.modbus_tx.send(ModbusCommand::WriteRegister(C_STATE_ADDR, param.cstate_gain_val));

                                if self.auto_reset_cstate {
                                    let delay: u64 = self.auto_reset_delay.trim().parse().unwrap_or(2);
                                    let _ = self.modbus_tx.send(ModbusCommand::WriteRegisterDelayed(C_STATE_ADDR, 0, delay));
                                }

                                self.current_step = CalibStep::Finished;
                            }
                        }

                        CalibStep::Finished => {
                            ui.colored_label(theme::SUCCESS, egui::RichText::new("✔  Calibration Complete!").heading());
                            ui.label(format!("Successfully calibrated {}", param.name));
                            ui.add_space(10.0);
                            self.show_current_value(ui);
                            ui.add_space(16.0);
                            if theme::primary_button(ui, "Start New Calibration").clicked() {
                                self.input_offset_val = "0".to_string();
                                self.input_gain_val = "".to_string();
                                self.current_step = CalibStep::SelectParam;
                            }
                        }
                    }
                });

                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    if theme::danger_button(ui, "Disconnect").clicked() {
                        let _ = self.modbus_tx.send(ModbusCommand::Disconnect);
                        self.is_connected = false;
                        self.current_step = CalibStep::SelectParam;
                        self.error_message = None;
                    }
                    if self.current_step != CalibStep::SelectParam && self.current_step != CalibStep::Finished {
                        if ui.button("Abort Calibration").clicked() {
                            self.current_step = CalibStep::SelectParam;
                            self.error_message = None;
                        }
                    }
                });
            }
        });
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([520.0, 620.0])
            .with_min_inner_size([420.0, 480.0])
            .with_icon(load_icon()),
        ..Default::default()
    };
    eframe::run_native(
        "Modbus Auto-Calibrator",
        options,
        Box::new(|cc| Box::new(CalibratorApp::new(cc))),
    )
}
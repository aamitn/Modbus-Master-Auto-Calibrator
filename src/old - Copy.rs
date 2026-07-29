use eframe::egui;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use tokio::runtime::Runtime;
use tokio_modbus::prelude::*;
use tokio_modbus::client::{tcp, rtu, Context};
use tokio_modbus::Slave;
use tokio_serial::SerialStream;

const C_STATE_ADDR: u16 = 190;
const TIMEOUT_SECS: u64 = 2; // Timeout for unresponsive devices

// --- Data Structures ---

#[derive(Clone, PartialEq)]
struct CalibParam {
    name: &'static str,
    offset_addr: u16,
    gain_addr: u16,
    cstate_offset_val: u16,
    cstate_gain_val: u16,
}

fn get_parameters() -> Vec<CalibParam> {
    vec![
        CalibParam { name: "V_RY", offset_addr: 191, gain_addr: 192, cstate_offset_val: 1, cstate_gain_val: 2 },
        CalibParam { name: "V_YB", offset_addr: 193, gain_addr: 194, cstate_offset_val: 3, cstate_gain_val: 4 },
        CalibParam { name: "V_BR", offset_addr: 195, gain_addr: 196, cstate_offset_val: 5, cstate_gain_val: 6 },
        CalibParam { name: "I_R", offset_addr: 197, gain_addr: 198, cstate_offset_val: 7, cstate_gain_val: 8 },
        CalibParam { name: "I_Y", offset_addr: 199, gain_addr: 200, cstate_offset_val: 9, cstate_gain_val: 10 },
        CalibParam { name: "I_B", offset_addr: 201, gain_addr: 202, cstate_offset_val: 11, cstate_gain_val: 12 },
        CalibParam { name: "V_CH", offset_addr: 203, gain_addr: 204, cstate_offset_val: 13, cstate_gain_val: 14 },
        CalibParam { name: "I_CH", offset_addr: 205, gain_addr: 206, cstate_offset_val: 15, cstate_gain_val: 16 },
        CalibParam { name: "V_BAT", offset_addr: 207, gain_addr: 208, cstate_offset_val: 17, cstate_gain_val: 18 },
        CalibParam { name: "I_BAT", offset_addr: 209, gain_addr: 210, cstate_offset_val: 19, cstate_gain_val: 20 },
        CalibParam { name: "V_LOAD", offset_addr: 211, gain_addr: 212, cstate_offset_val: 21, cstate_gain_val: 22 },
        CalibParam { name: "I_LOAD", offset_addr: 213, gain_addr: 214, cstate_offset_val: 23, cstate_gain_val: 24 },
        CalibParam { name: "T_BAT", offset_addr: 215, gain_addr: 216, cstate_offset_val: 25, cstate_gain_val: 26 },
    ]
}

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
    Disconnect,
}

pub enum AppEvent {
    Connected,
    ConnectionError(String),
    WriteError(String),
    Disconnected,
}

// --- Background Modbus Worker ---

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

// --- Helper for fetching COM Ports ---
fn fetch_available_ports() -> Vec<String> {
    tokio_serial::available_ports()
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.port_name)
        .collect()
}

// --- egui Application ---

struct CalibratorApp {
    modbus_tx: mpsc::Sender<ModbusCommand>,
    app_rx: mpsc::Receiver<AppEvent>,
    
    is_connected: bool,
    is_connecting: bool,
    error_message: Option<String>,
    
    is_tcp: bool,
    tcp_ip: String,
    enforce_tcp_device_id: bool,
    
    rtu_port: String,
    available_ports: Vec<String>, // Stores detected COM ports
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
}

impl CalibratorApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let (app_tx, app_rx) = mpsc::channel();
        let modbus_tx = spawn_modbus_worker(app_tx);
        
        let available_ports = fetch_available_ports();
        let initial_port = available_ports.first().cloned().unwrap_or_else(|| "COM1".to_string());
        
        Self {
            modbus_tx,
            app_rx,
            is_connected: false,
            is_connecting: false,
            error_message: None,
            is_tcp: true,
            tcp_ip: "127.0.0.1:502".to_string(),
            enforce_tcp_device_id: true,
            rtu_port: initial_port,
            available_ports,
            rtu_baud: "9600".to_string(),
            device_id: "1".to_string(),
            parameters: get_parameters(),
            selected_idx: 0,
            current_step: CalibStep::SelectParam,
            input_offset_val: "0".to_string(),
            input_gain_val: "".to_string(),
            scale_factor: "10".to_string(),
            auto_reset_cstate: true,
            auto_reset_delay: "2".to_string(),
        }
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
                }
                AppEvent::ConnectionError(err) => {
                    self.is_connecting = false;
                    self.error_message = Some(format!("Connection Failed: {}", err));
                }
                AppEvent::WriteError(err) => {
                    self.error_message = Some(format!("Write Error: {}", err));
                }
                AppEvent::Disconnected => {
                    self.is_connected = false;
                }
            }
        }

        ctx.request_repaint();

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Modbus Auto-Calibration Master");
            ui.separator();
            
            // --- ALWAYS VISIBLE SETTINGS ---
            ui.add_space(5.0);
            ui.horizontal(|ui| {
                ui.label("Global Modbus Scale Factor:");
                ui.text_edit_singleline(&mut self.scale_factor);
            });
            
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.auto_reset_cstate, "Auto-Reset C_State (Addr 190) to 0 after Calibration");
                if self.auto_reset_cstate {
                    ui.label("Delay (sec):");
                    ui.add(egui::TextEdit::singleline(&mut self.auto_reset_delay).desired_width(40.0));
                }
            });
            ui.add_space(5.0);
            ui.separator();

            if let Some(err) = &self.error_message {
                ui.colored_label(egui::Color32::RED, err);
                ui.add_space(10.0);
            }

            if !self.is_connected {
                ui.label("Select Connection Type:");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.is_tcp, true, "Modbus TCP");
                    ui.radio_value(&mut self.is_tcp, false, "Modbus RTU");
                });

                ui.add_space(10.0);

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
                    // --- RTU COM Port Editable Dropdown ---
                    ui.horizontal(|ui| {
                        ui.label("COM Port:");
                        // 1. Text input for manual entry
                        ui.add(egui::TextEdit::singleline(&mut self.rtu_port).desired_width(120.0));
                        
                        // 2. Dropdown arrow to select from detected ports
                        egui::ComboBox::from_id_source("com_port_dropdown")
                            .selected_text("▼")
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
                        
                        // 3. Refresh button to scan for new ports
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

                ui.add_space(20.0);
                
                if self.is_connecting {
                    ui.spinner();
                    ui.label("Connecting...");
                } else {
                    if ui.button("Connect").clicked() {
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
                                slave_id: parsed_id 
                            }
                        };
                        
                        let _ = self.modbus_tx.send(ModbusCommand::Connect(config));
                    }
                }
            } else {
                let param = self.parameters[self.selected_idx].clone();

                match self.current_step {
                    CalibStep::SelectParam => {
                        ui.label("Select Parameter to Calibrate:");
                        egui::ComboBox::from_label("")
                            .selected_text(param.name)
                            .show_ui(ui, |ui| {
                                for (i, p) in self.parameters.iter().enumerate() {
                                    ui.selectable_value(&mut self.selected_idx, i, p.name);
                                }
                            });

                        ui.add_space(20.0);
                        if ui.button("Start Calibration").clicked() {
                            self.current_step = CalibStep::Step1SetZeroExternally;
                            self.error_message = None;
                        }
                    }

                    CalibStep::Step1SetZeroExternally => {
                        ui.heading(format!("Calibrating: {}", param.name));
                        ui.label("Step 1: Externally make the actual reading value of the parameter 0.");
                        ui.add_space(10.0);
                        if ui.button("Done - Proceed to Step 2").clicked() {
                            self.current_step = CalibStep::Step2WriteOffsetValue;
                        }
                    }

                    CalibStep::Step2WriteOffsetValue => {
                        ui.label(format!("Step 2: Enter measured value for {} Offset", param.name));
                        ui.horizontal(|ui| {
                            ui.label("Offset Value:");
                            ui.text_edit_singleline(&mut self.input_offset_val);
                        });
                        
                        ui.add_space(10.0);
                        if ui.button(format!("Write Offset & Trigger C_State")).clicked() {
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
                        ui.label("Step 3: Set the actual parameter to the desired value externally.");
                        ui.add_space(10.0);
                        if ui.button("Done - Proceed to Step 4").clicked() {
                            self.current_step = CalibStep::Step4WriteGainValue;
                        }
                    }

                    CalibStep::Step4WriteGainValue => {
                        ui.label(format!("Step 4: Enter the desired measured value for {} Gain", param.name));
                        ui.horizontal(|ui| {
                            ui.label("Desired Value:");
                            ui.text_edit_singleline(&mut self.input_gain_val);
                        });
                        
                        ui.add_space(10.0);
                        if ui.button(format!("Write Gain & Trigger C_State")).clicked() {
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
                        ui.heading("Calibration Complete!");
                        ui.label(format!("Successfully calibrated {}", param.name));
                        ui.add_space(20.0);
                        if ui.button("Start New Calibration").clicked() {
                            self.input_offset_val = "0".to_string();
                            self.input_gain_val = "".to_string();
                            self.current_step = CalibStep::SelectParam;
                        }
                    }
                }
                
                ui.add_space(30.0);
                ui.separator();
                
                ui.horizontal(|ui| {
                    if ui.button("Disconnect").clicked() {
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
        viewport: egui::ViewportBuilder::default().with_inner_size([480.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Auto-Calibration Master",
        options,
        Box::new(|cc| Box::new(CalibratorApp::new(cc))),
    )
}
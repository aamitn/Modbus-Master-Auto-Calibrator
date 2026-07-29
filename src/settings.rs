//! Persisted application settings.
//!
//! On first run there is no settings file, so we build one from the sane
//! defaults below and write it next to the executable. On every subsequent
//! run we load that file. The user can also explicitly Save / Reload /
//! Export / Import from the File menu.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const SETTINGS_FILE_NAME: &str = "calibrator_settings.json";

#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct ConnectionSettings {
    pub is_tcp: bool,
    pub tcp_ip: String,
    pub enforce_tcp_device_id: bool,
    pub rtu_port: String,
    pub rtu_baud: String,
    pub device_id: String,
}

impl Default for ConnectionSettings {
    fn default() -> Self {
        Self {
            is_tcp: true,
            tcp_ip: "127.0.0.1:502".to_string(),
            enforce_tcp_device_id: true,
            rtu_port: "COM1".to_string(),
            rtu_baud: "9600".to_string(),
            device_id: "1".to_string(),
        }
    }
}

/// One row of the calibration register map. This used to be a compiled-in
/// constant (`get_parameters()` in main.rs) — it now lives in the settings
/// file too, seeded with those same sane defaults on first run, so the
/// register map can be inspected/edited/exported without a recompile.
#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct CalibParam {
    pub name: String,
    pub offset_addr: u16,
    pub gain_addr: u16,
    pub cstate_offset_val: u16,
    pub cstate_gain_val: u16,
    /// Register holding the live measured value for this parameter.
    /// An EXPLICIT mapping, not derivable from offset/gain addrs.
    /// 0 = not yet mapped (shown as "—" in the UI, polling skipped).
    pub current_val_addr: u16,
}

/// The known-good factory register map. This is what gets written to disk
/// the very first time the app runs (no settings file yet), and what "Reset
/// to Defaults" restores.
pub fn default_parameters() -> Vec<CalibParam> {
    fn p(name: &str, offset_addr: u16, gain_addr: u16, cstate_offset_val: u16, cstate_gain_val: u16, current_val_addr: u16) -> CalibParam {
        CalibParam {
            name: name.to_string(),
            offset_addr,
            gain_addr,
            cstate_offset_val,
            cstate_gain_val,
            current_val_addr,
        }
    }
    vec![
        p("V_RY",   191, 192, 1,  2,  2),
        p("V_YB",   193, 194, 3,  4,  3),
        p("V_BR",   195, 196, 5,  6,  4),
        p("I_R",    197, 198, 7,  8,  5),
        p("I_Y",    199, 200, 9,  10, 6),
        p("I_B",    201, 202, 11, 12, 7),
        p("V_CH",   203, 204, 13, 14, 8),
        p("I_CH",   205, 206, 15, 16, 9),
        p("V_BAT",  207, 208, 17, 18, 10),
        p("I_BAT",  209, 210, 19, 20, 11),
        p("V_LOAD", 211, 212, 21, 22, 12),
        p("I_LOAD", 213, 214, 23, 24, 13),
        p("T_BAT",  215, 216, 25, 26, 14),
    ]
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct AppSettings {
    pub connection: ConnectionSettings,
    pub scale_factor: String,
    pub auto_reset_cstate: bool,
    pub auto_reset_delay: String,
    pub dark_mode: bool,
    pub parameters: Vec<CalibParam>,
    /// Bumped whenever the on-disk schema changes, so future versions of the
    /// app can detect and migrate older settings files instead of silently
    /// misreading them.
    pub schema_version: u32,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            connection: ConnectionSettings::default(),
            scale_factor: "10".to_string(),
            auto_reset_cstate: true,
            auto_reset_delay: "2".to_string(),
            dark_mode: true,
            parameters: default_parameters(),
            schema_version: 1,
        }
    }
}

/// Where the settings file lives: next to the running executable. This
/// keeps the app portable (copy the folder, settings travel with it) which
/// is usually what's wanted for a plant-floor calibration tool.
fn settings_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(SETTINGS_FILE_NAME)))
        .unwrap_or_else(|| PathBuf::from(SETTINGS_FILE_NAME))
}

#[derive(Debug)]
pub enum SettingsLoadResult {
    /// File existed and parsed fine.
    Loaded,
    /// No file existed; a fresh one was generated with defaults.
    CreatedDefault,
    /// File existed but was corrupt/unreadable; defaults were used and the
    /// bad file was left in place (not overwritten) so it can be inspected.
    CorruptFellBackToDefault(String),
}

impl AppSettings {
    /// Loads settings from disk, creating the file with sane defaults if it
    /// doesn't exist yet. Returns both the settings and a status the UI can
    /// surface to the user (e.g. "Created new settings file").
    pub fn load_or_create() -> (Self, SettingsLoadResult) {
        let path = settings_path();

        match std::fs::read_to_string(&path) {
            Ok(contents) => match serde_json::from_str::<AppSettings>(&contents) {
                Ok(settings) => (settings, SettingsLoadResult::Loaded),
                Err(e) => {
                    let defaults = AppSettings::default();
                    (defaults, SettingsLoadResult::CorruptFellBackToDefault(e.to_string()))
                }
            },
            Err(_) => {
                let defaults = AppSettings::default();
                // Best-effort: generate the file now so it exists for next launch
                // and so the user can find/edit it immediately.
                let _ = defaults.save();
                (defaults, SettingsLoadResult::CreatedDefault)
            }
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        self.export_to(&settings_path())
    }

    pub fn export_to(&self, path: &Path) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(path, json)
    }

    pub fn import_from(path: &Path) -> Result<Self, String> {
        let contents = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        serde_json::from_str(&contents).map_err(|e| e.to_string())
    }

    pub fn settings_file_path() -> PathBuf {
        settings_path()
    }
}
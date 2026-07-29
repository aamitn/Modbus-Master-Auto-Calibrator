
# Modbus Auto-Calibration Master

A robust, cross-platform desktop application built in **Rust** using **`egui`/`eframe`** and **`tokio-modbus`**. Designed for industrial engineers and technicians to perform automated offset and gain calibration on metering devices and power electronics hardware via **Modbus TCP** or **Modbus RTU**.

## 🚀 Key Features

-   **Dual Communication Protocols:** Full native support for **Modbus TCP** (with optional Unit/Device ID enforcement) and **Modbus RTU** over serial ports.
    
-   **Auto-Detect COM Ports:** Automatically scans and lists available physical and virtual COM ports with an editable dropdown for custom port entry and a live refresh button.
    
-   **Guided Calibration Wizard:** Step-by-step wizard workflow (`Select Parameter` $\rightarrow$ `Zero Offset` $\rightarrow$ `Target Gain` $\rightarrow$ `Finished`).
    
-   **Live Register Polling:** Real-time background polling (every 500ms) of active registers, displaying both raw values and 10x downscaled engineering values.
    
-   **Automatic State Machine Resets:** Automatically writes and resets the control state register (`C_State` at address `190`) with configurable post-write delays.
    
-   **Persistent & Portable Settings:** Automatically saves connection parameters, scale factors, and custom register maps to a local `calibrator_settings.json` file placed right next to the executable.
    
-   **Custom Register Map Editor:** In-app visual editor to add, remove, or modify register mappings without touching code or recompiling.
    
-   **Polished Professional UI:** Features custom dark/light theme switching, status indicators, and embedded Windows resource icons.
    

## 🛠️ Tech Stack

-   **Language:** Rust (2024 Edition)
    
-   **GUI Framework:** `egui` / `eframe`
    
-   **Async Runtime & Modbus:** `tokio` & `tokio-modbus` / `tokio-serial`
    
-   **Configuration:** `serde` & `serde_json`
    
-   **File Dialogs:** `rfd`
    

## 📦 Building for Windows

To build a standalone, optimized release executable with an embedded Windows icon:

1.  Ensure you have an icon file saved as `assets/icon.ico`.
    
2.  Run the release compiler in your terminal:
    
    Bash
    
    ```
    cargo build --release
    
    ```
    
3.  Your compiled binary will be available at:
    
    Plaintext
    
    ```
    target\release\modbus_calibrator.exe
    
    ```
    

_(Note: The application automatically handles settings persistence locally. When moving or deploying the `.exe`, ensure it has read/write permissions in its directory to create and maintain `calibrator_settings.json`)._

## ⚙ Configuration & Register Map

On its first launch, the application generates a default configuration file (`calibrator_settings.json`) covering core electrical parameters:

-   **Voltages:** `V_RY`, `V_YB`, `V_BR`, `V_CH`, `V_BAT`, `V_LOAD`
    
-   **Currents:** `I_R`, `I_Y`, `I_B`, `I_CH`, `I_BAT`, `I_LOAD`
    
-   **Temperature:** `T_BAT`
    

You can customize offset addresses, gain addresses, C-state write values, and live-value polling addresses at runtime via **File $\rightarrow$ Edit Register Map**.

## 📄 License

Built for industrial automation, power electronics, and embedded R&D workflows.

use apollos_types::{
    AqiCondensed, CalendarCondensed, CondensedData, EphemerisCondensed, GbfsCondensed,
    GtfsCondensed, QueryInfo, TidalCondensed, WeatherCondensed,
};
use clap::Parser;
use eframe::egui;
use egui_material_icons::icons::*;
use log::info;
use paho_mqtt as mqtt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

#[derive(Serialize, Deserialize, Clone)]
struct Config {
    panels: [Vec<String>; 3],
    unassigned: Vec<String>,
    #[serde(default)]
    current_theme: String,
    #[serde(default)]
    mqtt_theme_sync: bool,
    #[serde(default = "default_theme_mqtt_host")]
    mqtt_theme_host: String,
    #[serde(default)]
    mqtt_theme_username: Option<String>,
    #[serde(default)]
    mqtt_theme_password: Option<String>,
    #[serde(default = "default_theme_mqtt_topic")]
    mqtt_theme_topic: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            panels: [vec![], vec![], vec![]],
            unassigned: vec![],
            current_theme: "Dark".to_string(),
            mqtt_theme_sync: false,
            mqtt_theme_host: default_theme_mqtt_host(),
            mqtt_theme_username: None,
            mqtt_theme_password: None,
            mqtt_theme_topic: default_theme_mqtt_topic(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Theme {
    name: String,
    background_color: [u8; 3],
    text_color: [u8; 3],
    accent_color: [u8; 3],
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            name: "Dark".to_string(),
            background_color: [40, 44, 52],
            text_color: [220, 223, 228],
            accent_color: [255, 180, 100],
        }
    }
}

fn create_default_themes() -> Vec<Theme> {
    vec![
        Theme {
            name: "Light".to_string(),
            background_color: [240, 240, 245],
            text_color: [60, 60, 70],
            accent_color: [100, 100, 180],
        },
        Theme {
            name: "Dark".to_string(),
            background_color: [40, 44, 52],
            text_color: [220, 223, 228],
            accent_color: [255, 180, 100],
        },
        Theme {
            name: "Solarized".to_string(),
            background_color: [0, 43, 54],
            text_color: [131, 148, 150],
            accent_color: [181, 137, 0],
        },
        Theme {
            name: "After Dark".to_string(),
            background_color: [32, 29, 101],
            text_color: [172, 171, 213],
            accent_color: [254, 243, 199],
        },
        Theme {
            name: "Her".to_string(),
            background_color: [101, 29, 29],
            text_color: [213, 171, 171],
            accent_color: [254, 243, 199],
        },
        Theme {
            name: "Forest".to_string(),
            background_color: [5, 46, 22],
            text_color: [134, 239, 172],
            accent_color: [254, 243, 199],
        },
        Theme {
            name: "Sky".to_string(),
            background_color: [8, 47, 73],
            text_color: [125, 211, 252],
            accent_color: [254, 243, 199],
        },
        Theme {
            name: "Clays".to_string(),
            background_color: [69, 26, 3],
            text_color: [245, 158, 11],
            accent_color: [254, 243, 199],
        },
        Theme {
            name: "Stones".to_string(),
            background_color: [41, 37, 36],
            text_color: [156, 163, 175],
            accent_color: [254, 243, 199],
        },
    ]
}

fn default_theme_mqtt_host() -> String {
    "localhost".to_string()
}

fn default_theme_mqtt_topic() -> String {
    "neiam/sync/theme".to_string()
}

#[derive(Debug, Parser, Clone)]
struct Args {
    // Data MQTT connection
    #[arg(long, default_value = "localhost", env = "MQTT_HOST")]
    mqtt_host: String,

    #[arg(long, env = "MQTT_USERNAME")]
    mqtt_username: String,

    #[arg(long, env = "MQTT_PASSWORD")]
    mqtt_password: String,

    #[arg(long, env = "MQTT_TOPIC")]
    mqtt_topic: String,

    // Theme MQTT connection
    #[arg(long, env = "MQTT_THEME_SYNC")]
    mqtt_theme_sync: Option<bool>,

    #[arg(long, default_value = "tcp://localhost:2883", env = "MQTT_THEME_HOST")]
    mqtt_theme_host: String,

    #[arg(long, env = "MQTT_THEME_USERNAME")]
    mqtt_theme_username: Option<String>,

    #[arg(long, env = "MQTT_THEME_PASSWORD")]
    mqtt_theme_password: Option<String>,

    #[arg(long, default_value = "neiam/sync/theme", env = "MQTT_THEME_TOPIC")]
    mqtt_theme_topic: String,
}

#[derive(Debug, Clone)]
struct DataEntry {
    content: CondensedData,
    query_info: Option<QueryInfo>,
}

struct ApollosKiosk {
    _args: Args,
    rx: Receiver<mqtt::Message>,
    data: HashMap<String, DataEntry>,
    config: Config,
    config_path: std::path::PathBuf,
    theme_rx: Receiver<String>,
    themes: Vec<Theme>,
    current_theme: String,
    show_theme_selector: bool,
    base_width: f32,
    base_height: f32,
}

impl ApollosKiosk {
    fn new(cc: &eframe::CreationContext<'_>, args: Args) -> Self {
        // Initialize material icons
        egui_material_icons::initialize(&cc.egui_ctx);

        let (tx, rx) = mpsc::channel();

        let config_path = dirs::config_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("apollos-kiosk")
            .join("config.toml");

        if let Some(parent) = config_path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        let config: Config = fs::read_to_string(&config_path)
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default();

        // Merge args with config (args take precedence)
        let mut config = config;
        if let Some(sync) = args.mqtt_theme_sync {
            config.mqtt_theme_sync = sync;
        }
        config.mqtt_theme_host = args.mqtt_theme_host.clone();
        if args.mqtt_theme_username.is_some() {
            config.mqtt_theme_username = args.mqtt_theme_username.clone();
        }
        if args.mqtt_theme_password.is_some() {
            config.mqtt_theme_password = args.mqtt_theme_password.clone();
        }
        config.mqtt_theme_topic = args.mqtt_theme_topic.clone();

        let themes = create_default_themes();
        let current_theme = config.current_theme.clone();

        // Apply initial theme
        let theme = themes
            .iter()
            .find(|t| t.name == current_theme)
            .cloned()
            .unwrap_or_default();

        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = egui::Color32::from_rgb(
            theme.background_color[0],
            theme.background_color[1],
            theme.background_color[2],
        );
        visuals.override_text_color = Some(egui::Color32::from_rgb(
            theme.text_color[0],
            theme.text_color[1],
            theme.text_color[2],
        ));
        cc.egui_ctx.set_visuals(visuals);

        let (theme_tx, theme_rx) = mpsc::channel();
        let mqtt_theme_sync = config.mqtt_theme_sync;

        // Setup data MQTT connection in a background thread
        let mqtt_args = args.clone();
        let ctx = cc.egui_ctx.clone();
        let value = args.clone();

        std::thread::spawn(move || {
            let args = value.clone();
            let create_opts = mqtt::CreateOptionsBuilder::new()
                .server_uri(format!("tcp://{}:1883", mqtt_args.mqtt_host))
                .client_id(args.mqtt_username)
                .finalize();

            let cli = mqtt::Client::new(create_opts).expect("Error creating MQTT client");
            let rx_mqtt = cli.start_consuming();

            let conn_opts = mqtt::ConnectOptionsBuilder::new()
                .keep_alive_interval(Duration::from_secs(20))
                .clean_session(true)
                .automatic_reconnect(Duration::from_secs(1), Duration::from_secs(30))
                .user_name(mqtt_args.mqtt_username)
                .password(mqtt_args.mqtt_password)
                .finalize();

            if let Err(e) = cli.connect(conn_opts) {
                eprintln!("Error connecting to MQTT: {:?}", e);
                return;
            }

            if let Err(e) = cli.subscribe(&mqtt_args.mqtt_topic, 1) {
                eprintln!("Error subscribing to topic: {:?}", e);
                return;
            }

            println!(
                "Data MQTT: Connected and subscribed to {}",
                mqtt_args.mqtt_topic
            );

            for msg in rx_mqtt.iter() {
                if let Some(msg) = msg {
                    println!("MQTT: Received message on topic '{}'", msg.topic());
                    let _ = tx.send(msg);
                    ctx.request_repaint();
                } else {
                    // None indicates a disconnection, but with auto-reconnect enabled
                    // the client will handle reconnection automatically
                    println!("Data MQTT: Disconnected, waiting for reconnection...");
                }
            }
        });

        // Setup separate MQTT connection for theme sync
        if mqtt_theme_sync {
            let theme_host = config.mqtt_theme_host.clone();
            let theme_username = config
                .mqtt_theme_username
                .clone()
                .unwrap_or_else(|| "kiosk-theme".to_string());
            let theme_password = config.mqtt_theme_password.clone().unwrap_or_default();
            let theme_topic = config.mqtt_theme_topic.clone();
            let theme_ctx = cc.egui_ctx.clone();

            std::thread::spawn(move || {
                let create_opts = mqtt::CreateOptionsBuilder::new()
                    .server_uri(theme_host)
                    .client_id(format!("{}-client", theme_username))
                    .finalize();

                let cli = mqtt::Client::new(create_opts).expect("Error creating theme MQTT client");
                let rx_mqtt = cli.start_consuming();

                let conn_opts = mqtt::ConnectOptionsBuilder::new()
                    .keep_alive_interval(Duration::from_secs(20))
                    .clean_session(true)
                    .automatic_reconnect(Duration::from_secs(1), Duration::from_secs(30))
                    .user_name(&theme_username)
                    .password(&theme_password)
                    .finalize();

                if let Err(e) = cli.connect(conn_opts) {
                    eprintln!("Error connecting to theme MQTT: {:?}", e);
                    return;
                }

                if let Err(e) = cli.subscribe(&theme_topic, 1) {
                    eprintln!(
                        "Error subscribing to theme topic '{}': {:?}",
                        theme_topic, e
                    );
                    return;
                }

                println!("Theme MQTT: Connected and subscribed to {}", theme_topic);

                for msg in rx_mqtt.iter() {
                    if let Some(msg) = msg {
                        println!("Theme MQTT: Received message on topic '{}'", msg.topic());

                        if msg.topic() == theme_topic {
                            if let Ok(json) =
                                serde_json::from_str::<serde_json::Value>(&msg.payload_str())
                            {
                                if let Some(theme_name) = json.get("theme").and_then(|t| t.as_str())
                                {
                                    println!("Theme MQTT: Parsed theme name: {}", theme_name);
                                    let _ = theme_tx.send(theme_name.to_string());
                                    theme_ctx.request_repaint();
                                }
                            }
                        }
                    } else {
                        // None indicates a disconnection, but with auto-reconnect enabled
                        // the client will handle reconnection automatically
                        println!("Theme MQTT: Disconnected, waiting for reconnection...");
                    }
                }
            });
        }

        Self {
            _args: args,
            rx,
            data: HashMap::new(),
            config,
            config_path,
            theme_rx,
            themes,
            current_theme,
            show_theme_selector: false,
            base_width: 1920.0,
            base_height: 1080.0,
        }
    }

    fn save_config(&self) {
        let mut cfg = self.config.clone();
        cfg.current_theme = self.current_theme.clone();
        if let Ok(s) = toml::to_string_pretty(&self.config) {
            let _ = fs::write(&self.config_path, s);
        }
    }

    fn render_data_item(ui: &mut egui::Ui, key: &str, content: &CondensedData) {
        let card_frame = egui::Frame::group(ui.style())
            .fill(ui.visuals().faint_bg_color)
            .rounding(8.0)
            .inner_margin(12.0)
            .outer_margin(egui::Margin::symmetric(0, 4));

        card_frame.show(ui, |ui| {
            ui.set_min_width(ui.available_width());

            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(key)
                        .strong()
                        .size(12.0)
                        .color(ui.visuals().weak_text_color()),
                );
            });
            ui.add_space(4.0);

            match content {
                CondensedData::Gtfs(routes) => {
                    for r in routes {
                        ui.group(|ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(&r.route).heading().strong());
                                ui.label(egui::RichText::new(&r.dest).size(16.0));
                            });
                            ui.label(
                                egui::RichText::new(&r.dir)
                                    .small()
                                    .color(ui.visuals().weak_text_color()),
                            );

                            ui.add_space(4.0);
                            ui.horizontal_wrapped(|ui| {
                                for time in &r.times {
                                    ui.label(
                                        egui::RichText::new(time)
                                            .monospace()
                                            .background_color(ui.visuals().extreme_bg_color),
                                    );
                                }
                            });
                        });
                        ui.add_space(4.0);
                    }
                }
                CondensedData::Gbfs(stations) => {
                    for s in stations {
                        ui.label(egui::RichText::new(&s.name).strong());
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            ui.label(format!("🚲 {}", s.avail));
                            ui.separator();
                            ui.label(format!("⚡ {}", s.avail_elec));
                            ui.separator();
                            ui.label(format!("🅿 {}", s.docks_avail));
                        });
                    }
                }
                CondensedData::Weather(reports) => {
                    for w in reports {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(format!("{:.0}°", w.temp))
                                    .size(32.0)
                                    .strong(),
                            );
                            ui.vertical(|ui| {
                                ui.label(egui::RichText::new(&w.weather).strong());
                                ui.label(format!("Feels like {:.0}°", w.feel));
                            });
                        });
                        ui.small(format!("Wind: {}mph | Hum: {}%", w.wind.speed, w.hum));
                    }
                }
                CondensedData::Calendar(events) => {
                    for e in events {
                        ui.label(egui::RichText::new(&e.description).strong());
                        ui.label(egui::RichText::new(&e.date_start).small().monospace());
                        ui.add_space(4.0);
                    }
                }
                CondensedData::Aqi(reports) => {
                    for a in reports {
                        ui.label(
                            egui::RichText::new(a.name.as_deref().unwrap_or("Unknown")).strong(),
                        );
                        ui.label(format!("Measurements: {}", a.measurements.len()));
                    }
                }
                CondensedData::Tidal(reports) => {
                    for t in reports {
                        if let Some(h) = &t.first_h {
                            ui.label(format!("⬆ High: {}", h));
                        }
                        if let Some(l) = &t.first_l {
                            ui.label(format!("⬇ Low: {}", l));
                        }
                    }
                }
                _ => {
                    ui.label("Data received...");
                }
            }
        });
    }
}

impl eframe::App for ApollosKiosk {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Check for incoming theme updates from MQTT
        if let Ok(theme_name) = self.theme_rx.try_recv() {
            let kiosk_theme = match theme_name.to_lowercase().as_str() {
                "light" | "light-soft" => "Light",
                "dark" | "dark-soft" | "dark-dimmed" => "Dark",
                "after-dark" => "After Dark",
                "her" => "Her",
                "forest" => "Forest",
                "sky" => "Sky",
                "clays" => "Clays",
                "stones" => "Stones",
                "solarized" => "Solarized",
                _ => "",
            };

            if !kiosk_theme.is_empty() && kiosk_theme != self.current_theme {
                self.current_theme = kiosk_theme.to_string();
                self.apply_theme(ctx);
                self.save_config();
                println!("Theme updated via MQTT to: {}", kiosk_theme);
            } else if kiosk_theme.is_empty() {
                eprintln!("Unknown theme from MQTT: {}, keeping current", theme_name);
            }
        }

        // Receive and parse any pending messages
        while let Ok(msg) = self.rx.try_recv() {
            let payload = msg.payload_str();

            if let Ok(raw_map) =
                serde_json::from_str::<HashMap<String, serde_json::Value>>(&payload)
            {
                if let Ok(pretty) = serde_json::to_string_pretty(&raw_map) {
                    println!("MQTT: Received valid data map:\n{}", pretty);
                }

                for (key, value) in raw_map {
                    if let Some(entry) = self.parse_data_entry(&key, &value) {
                        // Check if this is a new key
                        if !self.data.contains_key(&key) {
                            // Check if it's not already assigned to any panel or unassigned list
                            let is_assigned = self.config.panels.iter().any(|p| p.contains(&key))
                                || self.config.unassigned.contains(&key);

                            if !is_assigned {
                                println!("  - Adding {} to unassigned", key);
                                self.config.unassigned.push(key.clone());
                                self.save_config();
                            }
                        }

                        self.data.insert(key, entry);
                    } else {
                        println!("  - Failed to parse data for key: {}", key);
                    }
                }
            } else {
                println!("MQTT: Received payload that is not a Map: {}", payload);
            }
        }

        egui::TopBottomPanel::bottom("top_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Apollos Kiosk");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Theme selector button
                    if ui
                        .button(egui::RichText::new(ICON_PALETTE).size(20.0))
                        .clicked()
                    {
                        self.show_theme_selector = !self.show_theme_selector;
                    }

                    ui.separator();
                    ui.label(format!("{} data feeds", self.data.len()));
                });
            });
        });

        // Theme selector window
        if self.show_theme_selector {
            self.render_theme_selector(ctx);
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                // Show unassigned items at the top
                if !self.config.unassigned.is_empty() {
                    egui::Frame::group(ui.style())
                        .fill(ui.visuals().extreme_bg_color)
                        .rounding(8.0)
                        .inner_margin(12.0)
                        .outer_margin(8.0)
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new("📦 Unassigned Feeds")
                                    .heading()
                                    .strong(),
                            );
                            ui.add_space(8.0);

                            ui.horizontal_wrapped(|ui| {
                                let mut to_move = None;

                                for (idx, key) in self.config.unassigned.iter().enumerate() {
                                    ui.menu_button(format!("📌 {}", key), |ui| {
                                        ui.label(egui::RichText::new("Assign to panel:").strong());
                                        ui.separator();
                                        if ui.button("Panel 1 (Left)").clicked() {
                                            to_move = Some((idx, 0));
                                            ui.close_menu();
                                        }
                                        if ui.button("Panel 2 (Center)").clicked() {
                                            to_move = Some((idx, 1));
                                            ui.close_menu();
                                        }
                                        if ui.button("Panel 3 (Right)").clicked() {
                                            to_move = Some((idx, 2));
                                            ui.close_menu();
                                        }
                                    });
                                }

                                if let Some((idx, panel_idx)) = to_move {
                                    let key = self.config.unassigned.remove(idx);
                                    self.config.panels[panel_idx].push(key);
                                    self.save_config();
                                }
                            });
                        });
                    ui.add_space(8.0);
                }

                // 3-pane layout
                ui.columns(3, |columns| {
                    for (panel_idx, column) in columns.iter_mut().enumerate() {
                        self.render_panel(column, panel_idx);
                    }
                });
            });
        });
    }
}

impl ApollosKiosk {
    fn get_scale_factor(&self, ctx: &egui::Context) -> f32 {
        let viewport_size = ctx.screen_rect().size();
        let scale_x = viewport_size.x / self.base_width;
        let scale_y = viewport_size.y / self.base_height;
        scale_x.min(scale_y)
    }

    fn render_panel(&mut self, ui: &mut egui::Ui, panel_idx: usize) {
        let _panel_names = ["Left Panel", "Center Panel", "Right Panel"];

        egui::Frame::group(ui.style())
            .fill(ui.visuals().panel_fill)
            .corner_radius(8.0)
            .inner_margin(8.0)
            .show(ui, |ui| {
                ui.set_min_height(ui.available_height());

                ui.horizontal(|ui| {
                    // ui.label(egui::RichText::new(panel_names[panel_idx]).strong().size(16.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(format!("{}", self.config.panels[panel_idx].len()))
                                .weak()
                                .small(),
                        );
                    });
                });
                ui.separator();
                ui.add_space(4.0);

                let keys = self.config.panels[panel_idx].clone();
                let mut to_remove = None;
                let mut to_move = None;

                let scale = self.get_scale_factor(ui.ctx());
                for (idx, key) in keys.iter().enumerate() {
                    if let Some(entry) = self.data.get(key) {
                        self.render_large_card(
                            ui,
                            key,
                            entry,
                            panel_idx,
                            idx,
                            &mut to_remove,
                            &mut to_move,
                            scale,
                        );
                    }
                }

                // Handle card removal
                if let Some(idx) = to_remove {
                    let key = self.config.panels[panel_idx].remove(idx);
                    self.config.unassigned.push(key);
                    self.save_config();
                }

                // Handle card movement
                if let Some((idx, target_panel)) = to_move {
                    let key = self.config.panels[panel_idx].remove(idx);
                    self.config.panels[target_panel].push(key);
                    self.save_config();
                }
            });
    }

    fn render_large_card(
        &self,
        ui: &mut egui::Ui,
        key: &str,
        entry: &DataEntry,
        panel_idx: usize,
        card_idx: usize,
        to_remove: &mut Option<usize>,
        to_move: &mut Option<(usize, usize)>,
        scale: f32,
    ) {
        let card_frame = egui::Frame::group(ui.style())
            .fill(ui.visuals().faint_bg_color)
            .stroke(egui::Stroke::new(
                1.0,
                ui.visuals().widgets.noninteractive.bg_stroke.color,
            ))
            .corner_radius(12.0)
            .inner_margin(16.0)
            .outer_margin(egui::Margin::symmetric(0, 8));

        card_frame.show(ui, |ui| {
            ui.set_min_width(ui.available_width());

            // Card header with title and controls
            ui.horizontal(|ui| {
                // Use query name if available, otherwise fall back to key
                let display_name = entry
                    .query_info
                    .as_ref()
                    .map(|q| q.name.as_str())
                    .unwrap_or(key);

                ui.label(
                    egui::RichText::new(display_name)
                        .heading()
                        .strong()
                        .size(18.0 * scale),
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(key).weak().small());
                    ui.add_space(8.0);
                    ui.menu_button("⋮", |ui| {
                        if ui.button("🗑 Unassign").clicked() {
                            *to_remove = Some(card_idx);
                            ui.close();
                        }
                        ui.separator();
                        ui.label("Move to:");
                        for (idx, name) in ["Left", "Center", "Right"].iter().enumerate() {
                            if idx != panel_idx {
                                if ui.button(format!("➜ {}", name)).clicked() {
                                    *to_move = Some((card_idx, idx));
                                    ui.close();
                                }
                            }
                        }
                    });
                });
            });

            ui.add_space(12.0);
            ui.separator();
            ui.add_space(12.0);

            // Card content - use the existing render_data_item logic but inline
            match &entry.content {
                CondensedData::Gtfs(routes) => self.render_gtfs_card(ui, routes, scale),
                CondensedData::Gbfs(stations) => self.render_gbfs_card(ui, stations, scale),
                CondensedData::Weather(reports) => self.render_weather_card(ui, reports, scale),
                CondensedData::Calendar(events) => self.render_calendar_card(ui, events, scale),
                CondensedData::Aqi(reports) => self.render_aqi_card(ui, reports, scale),
                CondensedData::Tidal(reports) => self.render_tidal_card(ui, reports, scale),
                CondensedData::Ephem(reports) => self.render_ephem_card(ui, reports, scale),
                _ => {
                    ui.label(
                        egui::RichText::new("Data type not yet supported in card view").weak(),
                    );
                }
            }
        });
    }

    fn get_current_theme(&self) -> Theme {
        self.themes
            .iter()
            .find(|t| t.name == self.current_theme)
            .cloned()
            .unwrap_or_default()
    }

    fn apply_theme(&self, ctx: &egui::Context) {
        let theme = self.get_current_theme();
        let mut visuals = egui::Visuals::dark();

        visuals.panel_fill = egui::Color32::from_rgb(
            theme.background_color[0],
            theme.background_color[1],
            theme.background_color[2],
        );

        visuals.extreme_bg_color = egui::Color32::from_rgb(
            theme.background_color[0].saturating_add(10),
            theme.background_color[1].saturating_add(10),
            theme.background_color[2].saturating_add(10),
        );

        visuals.override_text_color = Some(egui::Color32::from_rgb(
            theme.text_color[0],
            theme.text_color[1],
            theme.text_color[2],
        ));

        ctx.set_visuals(visuals);
    }

    fn parse_data_entry(&self, key: &str, value: &serde_json::Value) -> Option<DataEntry> {
        // Check if this is wrapped format (has both "data" and "query" fields)
        if let Some(obj) = value.as_object() {
            if obj.contains_key("data") && obj.contains_key("query") {
                println!("  - Detected wrapped format for key: {}", key);

                // Extract query info
                let query_info = serde_json::from_value::<QueryInfo>(obj["query"].clone()).ok()?;

                // Parse the data based on key prefix
                let data_value = &obj["data"];
                let content = self.parse_content_by_prefix(key, data_value)?;

                println!("  - Successfully parsed wrapped data");
                return Some(DataEntry {
                    content,
                    query_info: Some(query_info),
                });
            }
        }

        // Legacy unwrapped format
        println!("  - Parsing legacy format for key: {}", key);
        let content = self.parse_content_by_prefix(key, value)?;
        println!("  - Successfully parsed legacy content");

        Some(DataEntry {
            content,
            query_info: None,
        })
    }

    fn parse_content_by_prefix(
        &self,
        key: &str,
        value: &serde_json::Value,
    ) -> Option<CondensedData> {
        // Handle empty data (like gtfs-1 in payload)
        if value.is_null() || (value.is_object() && value.as_object()?.is_empty()) {
            println!("  - Skipping empty data for key: {}", key);
            return None;
        }

        // Handle array data (check if empty)
        if let Some(arr) = value.as_array() {
            if arr.is_empty() {
                println!("  - Skipping empty array for key: {}", key);
                return None;
            }
        }

        let content = if key.starts_with("gtfs-") {
            serde_json::from_value::<Vec<GtfsCondensed>>(value.clone())
                .ok()
                .map(CondensedData::Gtfs)
        } else if key.starts_with("gbfs-") {
            serde_json::from_value::<Vec<GbfsCondensed>>(value.clone())
                .ok()
                .map(CondensedData::Gbfs)
        } else if key.starts_with("weather-") {
            serde_json::from_value::<Vec<WeatherCondensed>>(value.clone())
                .ok()
                .map(CondensedData::Weather)
        } else if key.starts_with("aqi-") {
            serde_json::from_value::<Vec<AqiCondensed>>(value.clone())
                .ok()
                .map(CondensedData::Aqi)
        } else if key.starts_with("ephem-") {
            serde_json::from_value::<Vec<EphemerisCondensed>>(value.clone())
                .ok()
                .map(CondensedData::Ephem)
        } else if key.starts_with("cal-") {
            serde_json::from_value::<Vec<CalendarCondensed>>(value.clone())
                .ok()
                .map(CondensedData::Calendar)
        } else if key.starts_with("tidal-") {
            serde_json::from_value::<Vec<TidalCondensed>>(value.clone())
                .ok()
                .map(CondensedData::Tidal)
                .or_else(|| {
                    if let Err(e) = serde_json::from_value::<Vec<TidalCondensed>>(value.clone()) {
                        println!("  - Tidal parse error: {}", e);
                    }
                    None
                })
        } else if key.starts_with("cronos-") {
            Some(CondensedData::Cronos(value.clone()))
        } else if key.starts_with("gitlab-") {
            Some(CondensedData::Gitlab(value.clone()))
        } else if key.starts_with("pkg-") {
            Some(CondensedData::Packages(value.clone()))
        } else if key.starts_with("const-") {
            Some(CondensedData::Const(value.clone()))
        } else {
            None
        }?;

        // Debug success for tidal
        if key.starts_with("tidal-") {
            println!("  - Tidal parse result: Success");
        }

        Some(content)
    }

    fn render_theme_selector(&mut self, ctx: &egui::Context) {
        let mut theme_changed = false;
        let mut new_theme = String::new();
        let mut show_selector = self.show_theme_selector;
        let mut show_mqtt_config = false;

        egui::Window::new("Theme Selector")
            .open(&mut show_selector)
            .resizable(false)
            .show(ctx, |ui| {
                ui.heading("Choose Theme");
                ui.separator();
                ui.add_space(10.0);

                for theme in &self.themes {
                    let is_selected = theme.name == self.current_theme;
                    if ui.selectable_label(is_selected, &theme.name).clicked() {
                        new_theme = theme.name.clone();
                        theme_changed = true;
                    }
                }

                ui.add_space(10.0);
                ui.separator();

                let old_sync = self.config.mqtt_theme_sync;
                if ui
                    .checkbox(
                        &mut self.config.mqtt_theme_sync,
                        "Sync via MQTT (neiam/sync/theme)",
                    )
                    .changed()
                {
                    self.save_config();
                    if self.config.mqtt_theme_sync != old_sync {
                        ui.label(
                            egui::RichText::new("⚠ Restart app to apply MQTT sync changes")
                                .color(egui::Color32::from_rgb(255, 180, 100)),
                        );
                    }
                }

                if self.config.mqtt_theme_sync {
                    if ui.button("⚙ Configure Theme MQTT...").clicked() {
                        show_mqtt_config = true;
                    }
                }

                ui.add_space(5.0);
            });

        self.show_theme_selector = show_selector;

        // Theme MQTT configuration window
        if show_mqtt_config {
            egui::Window::new("Theme MQTT Configuration")
                .resizable(false)
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.heading("Theme Sync MQTT Settings");
                    ui.separator();
                    ui.add_space(10.0);

                    ui.horizontal(|ui| {
                        ui.label("Host:");
                        ui.text_edit_singleline(&mut self.config.mqtt_theme_host);
                    });

                    ui.horizontal(|ui| {
                        ui.label("Username:");
                        let mut username =
                            self.config.mqtt_theme_username.clone().unwrap_or_default();
                        if ui.text_edit_singleline(&mut username).changed() {
                            self.config.mqtt_theme_username = if username.is_empty() {
                                None
                            } else {
                                Some(username)
                            };
                        }
                    });

                    ui.horizontal(|ui| {
                        ui.label("Password:");
                        let mut password =
                            self.config.mqtt_theme_password.clone().unwrap_or_default();
                        if ui
                            .add(egui::TextEdit::singleline(&mut password).password(true))
                            .changed()
                        {
                            self.config.mqtt_theme_password = if password.is_empty() {
                                None
                            } else {
                                Some(password)
                            };
                        }
                    });

                    ui.horizontal(|ui| {
                        ui.label("Topic:");
                        ui.text_edit_singleline(&mut self.config.mqtt_theme_topic);
                    });

                    ui.add_space(10.0);
                    ui.separator();

                    if ui.button("💾 Save").clicked() {
                        self.save_config();
                        ui.label(
                            egui::RichText::new("⚠ Restart app to apply changes")
                                .color(egui::Color32::from_rgb(255, 180, 100)),
                        );
                    }

                    ui.add_space(5.0);
                });
        }

        if theme_changed {
            self.current_theme = new_theme;
            self.apply_theme(ctx);
            self.save_config();
        }
    }

    fn mode_icon(mode: &apollos_types::GtfsMode) -> &'static str {
        match mode {
            apollos_types::GtfsMode::LightRail => ICON_TRAM,
            apollos_types::GtfsMode::Subway => ICON_SUBWAY,
            apollos_types::GtfsMode::Rail => ICON_TRAIN,
            apollos_types::GtfsMode::Bus => ICON_DIRECTIONS_BUS,
            apollos_types::GtfsMode::Ferry => ICON_DIRECTIONS_BOAT,
            apollos_types::GtfsMode::CableCar => ICON_CABLE_CAR,
            apollos_types::GtfsMode::Gondola => ICON_GONDOLA_LIFT,
            apollos_types::GtfsMode::Funicular => ICON_FUNICULAR,
            apollos_types::GtfsMode::Trolleybus => ICON_DIRECTIONS_BUS_FILLED,
            apollos_types::GtfsMode::Monorail => ICON_RAILWAY_ALERT,
            apollos_types::GtfsMode::Unknown => ICON_DIRECTIONS_TRANSIT,
        }
    }

    fn render_gtfs_card(&self, ui: &mut egui::Ui, routes: &[GtfsCondensed], scale: f32) {
        let accent = self.get_current_theme().accent_color;
        let accent_color = egui::Color32::from_rgb(accent[0], accent[1], accent[2]);

        for r in routes {
            egui::Frame::NONE
                .fill(ui.visuals().extreme_bg_color)
                .corner_radius(8.0 * scale)
                .inner_margin(12.0 * scale)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        // Mode icon
                        ui.label(egui::RichText::new(Self::mode_icon(&r.mode)).size(100.0 * scale));
                        ui.add_space(8.0 * scale);

                        // Route number/name
                        ui.label(
                            egui::RichText::new(&r.route)
                                .heading()
                                .strong()
                                .size(96.0 * scale)
                                .color(accent_color),
                        );
                        ui.add_space(8.0 * scale);

                        // Destination info
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new(&r.dest).size(32.0 * scale));
                            ui.label(
                                egui::RichText::new(&r.dir)
                                    .size(28.0 * scale)
                                    .color(ui.visuals().weak_text_color()),
                            );
                        });
                    });

                    ui.add_space(8.0 * scale);

                    // Determine which times to show (prioritize live times)
                    let times_to_show: Vec<(String, bool)> = if let Some(live_times) = &r.times_live
                    {
                        // Filter out None values and take up to 2
                        live_times
                            .iter()
                            .filter_map(|t| t.as_ref())
                            .take(2)
                            .map(|t| (t.clone(), true)) // true = live
                            .collect()
                    } else {
                        // Use scheduled times if no live times available
                        r.times
                            .iter()
                            .take(2)
                            .map(|t| (t.clone(), false)) // false = scheduled
                            .collect()
                    };

                    if !times_to_show.is_empty() {
                        // ui.label(egui::RichText::new("Next arrivals:").strong().size(14.0 * scale));
                        // ui.add_space(4.0 * scale);

                        ui.horizontal_wrapped(|ui| {
                            for (time, is_live) in times_to_show {
                                ui.horizontal(|ui| {
                                    // Icon to indicate live vs scheduled
                                    let (icon, color) = if is_live {
                                        (ICON_RADIO, egui::Color32::from_rgb(76, 175, 80)) // Green for live
                                    } else {
                                        (ICON_SCHEDULE, ui.visuals().weak_text_color()) // Gray for scheduled
                                    };

                                    ui.label(
                                        egui::RichText::new(icon).size(14.0 * scale).color(color),
                                    );
                                    ui.label(
                                        egui::RichText::new(&time)
                                            .monospace()
                                            .size(16.0 * scale)
                                            .color(ui.visuals().strong_text_color()), // .background_color(ui.visuals().code_bg_color)
                                    );
                                });
                            }
                        });
                    }
                });
            ui.add_space(8.0 * scale);
        }
    }

    fn render_gbfs_card(&self, ui: &mut egui::Ui, stations: &[GbfsCondensed], scale: f32) {
        let accent = self.get_current_theme().accent_color;
        let accent_color = egui::Color32::from_rgb(accent[0], accent[1], accent[2]);

        for s in stations {
            egui::Frame::NONE
                .fill(ui.visuals().extreme_bg_color)
                .corner_radius(8.0 * scale)
                .inner_margin(12.0 * scale)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    // ui.label(egui::RichText::new(&s.name).strong().size(16.0 * scale));
                    // ui.add_space(8.0 * scale);

                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(ICON_PEDAL_BIKE).size(100.0 * scale));
                        ui.label(
                            egui::RichText::new(format!("{}", s.avail_std))
                                .strong()
                                .color(accent_color)
                                .size(92.0 * scale),
                        );
                        ui.separator();
                        ui.label(egui::RichText::new(ICON_ELECTRIC_BIKE).size(100.0 * scale));
                        ui.label(
                            egui::RichText::new(format!("{}", s.avail_elec))
                                .strong()
                                .color(accent_color)
                                .size(92.0 * scale),
                        );
                        ui.separator();
                        ui.label(egui::RichText::new(ICON_LOCAL_PARKING).size(100.0 * scale));
                        ui.label(
                            egui::RichText::new(format!("{}", s.docks_avail))
                                .strong()
                                .color(accent_color)
                                .size(92.0 * scale),
                        );
                    });
                });
            ui.add_space(8.0 * scale);
        }
    }

    fn render_weather_card(&self, ui: &mut egui::Ui, reports: &[WeatherCondensed], scale: f32) {
        for w in reports {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("{:.0}°", w.temp))
                        .size(56.0 * scale)
                        .strong(),
                );
                ui.add_space(12.0 * scale);
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new(&w.weather).strong().size(22.0 * scale));
                    ui.label(
                        egui::RichText::new(format!("Feels like {:.0}°", w.feel))
                            .size(16.0 * scale),
                    );
                    ui.add_space(4.0 * scale);
                    ui.label(
                        egui::RichText::new(format!("💨 {}mph  💧 {}%", w.wind.speed, w.hum))
                            .size(14.0 * scale)
                            .weak(),
                    );
                });
            });
        }
    }

    fn render_calendar_card(&self, ui: &mut egui::Ui, events: &[CalendarCondensed], scale: f32) {
        for e in events {
            egui::Frame::NONE
                .fill(ui.visuals().extreme_bg_color)
                .corner_radius(8.0 * scale)
                .inner_margin(12.0 * scale)
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(&e.description)
                            .strong()
                            .size(16.0 * scale),
                    );
                    ui.add_space(4.0 * scale);
                    ui.label(
                        egui::RichText::new(&e.date_start)
                            .monospace()
                            .size(14.0 * scale)
                            .weak(),
                    );
                });
            ui.add_space(8.0 * scale);
        }
    }

    fn render_aqi_card(&self, ui: &mut egui::Ui, reports: &[AqiCondensed], scale: f32) {
        for a in reports {
            ui.label(
                egui::RichText::new(a.name.as_deref().unwrap_or("Unknown"))
                    .strong()
                    .size(16.0 * scale),
            );
            ui.add_space(4.0 * scale);
            ui.label(
                egui::RichText::new(format!("📊 {} measurements", a.measurements.len()))
                    .size(14.0 * scale),
            );
            ui.add_space(8.0 * scale);
        }
    }

    fn render_tidal_card(&self, ui: &mut egui::Ui, reports: &[TidalCondensed], scale: f32) {
        for t in reports {
            if let Some(h) = &t.first_h {
                ui.label(egui::RichText::new(format!("⬆ High: {}", h)).size(16.0 * scale));
                ui.add_space(4.0 * scale);
            }
            if let Some(l) = &t.first_l {
                ui.label(egui::RichText::new(format!("⬇ Low: {}", l)).size(16.0 * scale));
                ui.add_space(4.0 * scale);
            }
        }
    }

    fn render_ephem_card(&self, ui: &mut egui::Ui, reports: &[EphemerisCondensed], scale: f32) {
        for ephem in reports {
            egui::Frame::NONE
                .fill(ui.visuals().extreme_bg_color)
                .corner_radius(8.0 * scale)
                .inner_margin(12.0 * scale)
                .show(ui, |ui| {
                    // Display the name (location/body)
                    ui.label(egui::RichText::new(&ephem.name).strong().size(18.0 * scale));
                    ui.add_space(8.0 * scale);

                    // Display each period in a nice format
                    for (key, value) in &ephem.periods {
                        // Choose appropriate emoji/icon based on the key
                        let icon = if key.to_lowercase().contains("sunrise") {
                            "🌅"
                        } else if key.to_lowercase().contains("sunset") {
                            "🌇"
                        } else if key.to_lowercase().contains("moonrise") {
                            "🌙↑"
                        } else if key.to_lowercase().contains("moonset") {
                            "🌙↓"
                        } else if key.to_lowercase().contains("moon") {
                            "🌙"
                        } else if key.to_lowercase().contains("sun") {
                            "☀️"
                        } else {
                            "🔭"
                        };

                        // Format the key to be more readable
                        let formatted_key = key
                            .replace('_', " ")
                            .split_whitespace()
                            .map(|word| {
                                let mut chars = word.chars();
                                match chars.next() {
                                    None => String::new(),
                                    Some(first) => {
                                        first.to_uppercase().collect::<String>() + chars.as_str()
                                    }
                                }
                            })
                            .collect::<Vec<_>>()
                            .join(" ");

                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(icon).size(16.0 * scale));
                            ui.label(
                                egui::RichText::new(format!("{}: ", formatted_key))
                                    .size(14.0 * scale),
                            );
                            ui.label(egui::RichText::new(value).strong().size(14.0 * scale));
                        });
                        ui.add_space(4.0 * scale);
                    }
                });
            ui.add_space(8.0 * scale);
        }
    }
}

fn main() -> eframe::Result {
    // Try to load .env from XDG config directory
    if let Some(config_dir) = dirs::config_dir() {
        let env_path = config_dir.join("apollos-kiosk").join(".env");
        if env_path.exists() {
            if let Err(e) = dotenvy::from_path(&env_path) {
                info!(
                    "Warning: Failed to load .env from {}: {}",
                    env_path.display(),
                    e
                );
            } else {
                info!("Loaded environment from {}", env_path.display());
            }
        } else {
            println!(
                "Did not load env, path must be created: {}",
                env_path.display()
            )
        }
    }

    let args = Args::parse();

    let mut options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1920.0, 1080.0]),
        ..Default::default()
    };

    let d = eframe::icon_data::from_png_bytes(include_bytes!("../assets/logo.png"))
        .expect("The icon data must be valid");
    options.viewport.icon = Some(Arc::new(d));

    eframe::run_native(
        "Apollos Kiosk",
        options,
        Box::new(|cc| Ok(Box::new(ApollosKiosk::new(cc, args)))),
    )
}

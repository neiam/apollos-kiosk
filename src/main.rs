use clap::Parser;
use eframe::egui;
use paho_mqtt as mqtt;
use egui_material_icons::icons::*;
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, self};
use std::time::Duration;
use serde::{Serialize, Deserialize};
use std::fs;
use apollos_types::{
    CondensedData, MaybeWrappedData, WrappedData, QueryInfo,
    GtfsCondensed, GbfsCondensed, WeatherCondensed, 
    AqiCondensed, EphemerisCondensed, CalendarCondensed, TidalCondensed
};

#[derive(Serialize, Deserialize, Default, Clone)]
struct Config {
    panels: [Vec<String>; 3],
    unassigned: Vec<String>,
}

#[derive(Debug, Parser, Clone)]
struct Args {
    #[arg(long, default_value = "localhost", env = "MQTT_HOST")]
    mqtt_host: String,

    #[arg(long, env = "MQTT_USERNAME")]
    mqtt_username: String,

    #[arg(long, env = "MQTT_PASSWORD")]
    mqtt_password: String,

    #[arg(long, env = "MQTT_TOPIC")]
    mqtt_topic: String,
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

        let config = fs::read_to_string(&config_path)
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default();

        // Setup MQTT connection in a background thread
        let mqtt_args = args.clone();
        let ctx = cc.egui_ctx.clone();
        let value = args.clone();

        std::thread::spawn(move || {
            let args = value.clone();
            let create_opts = mqtt::CreateOptionsBuilder::new()
                .server_uri(format!("tcp://{}:1883", mqtt_args.mqtt_host))
                .client_id(args.mqtt_username)
                .finalize();

            let mut cli = mqtt::Client::new(create_opts).expect("Error creating MQTT client");
            let rx_mqtt = cli.start_consuming();

            let conn_opts = mqtt::ConnectOptionsBuilder::new()
                .keep_alive_interval(Duration::from_secs(20))
                .clean_session(true)
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

            for msg in rx_mqtt.iter() {
                if let Some(msg) = msg {
                    println!("MQTT: Received message on topic '{}'", msg.topic());
                    let _ = tx.send(msg);
                    ctx.request_repaint();
                }
            }
        });

        Self {
            _args: args,
            rx,
            data: HashMap::new(),
            config,
            config_path,
        }
    }

    fn save_config(&self) {
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
                ui.label(egui::RichText::new(key).strong().size(12.0).color(ui.visuals().weak_text_color()));
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
                            ui.label(egui::RichText::new(&r.dir).small().color(ui.visuals().weak_text_color()));
                            
                            ui.add_space(4.0);
                            ui.horizontal_wrapped(|ui| {
                                for time in &r.times {
                                    ui.label(egui::RichText::new(time).monospace().background_color(ui.visuals().extreme_bg_color));
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
                            ui.label(egui::RichText::new(format!("{:.0}°", w.temp)).size(32.0).strong());
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
                        ui.label(egui::RichText::new(&a.name).strong());
                        ui.label(format!("Measurements: {}", a.measurements.len()));
                    }
                }
                CondensedData::Tidal(reports) => {
                    for t in reports {
                        if let Some(h) = &t.first_high { ui.label(format!("⬆ High: {}", h)); }
                        if let Some(l) = &t.first_low { ui.label(format!("⬇ Low: {}", l)); }
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
        // Receive and parse any pending messages
        while let Ok(msg) = self.rx.try_recv() {
            let payload = msg.payload_str();

            if let Ok(raw_map) = serde_json::from_str::<HashMap<String, serde_json::Value>>(&payload) {
                for (key, value) in raw_map {
                    // Try to parse as wrapped data first (new format)
                    let (content, query_info) = if let Ok(wrapped) = serde_json::from_value::<WrappedData>(value.clone()) {
                        (Some(wrapped.data), Some(wrapped.query))
                    } else {
                        // Fall back to unwrapped format (old format)
                        let parsed = if key.starts_with("gtfs-") {
                            serde_json::from_value::<Vec<GtfsCondensed>>(value.clone()).ok().map(CondensedData::Gtfs)
                        } else if key.starts_with("gbfs-") {
                            serde_json::from_value::<Vec<GbfsCondensed>>(value.clone()).ok().map(CondensedData::Gbfs)
                        } else if key.starts_with("weather-") {
                            serde_json::from_value::<Vec<WeatherCondensed>>(value.clone()).ok().map(CondensedData::Weather)
                        } else if key.starts_with("aqi-") {
                            serde_json::from_value::<Vec<AqiCondensed>>(value.clone()).ok().map(CondensedData::Aqi)
                        } else if key.starts_with("ephem-") {
                            serde_json::from_value::<Vec<EphemerisCondensed>>(value.clone()).ok().map(CondensedData::Ephem)
                        } else if key.starts_with("cal-") {
                            serde_json::from_value::<Vec<CalendarCondensed>>(value.clone()).ok().map(CondensedData::Calendar)
                        } else if key.starts_with("tidal-") {
                            serde_json::from_value::<Vec<TidalCondensed>>(value.clone()).ok().map(CondensedData::Tidal)
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
                        };
                        (parsed, None)
                    };

                    if let Some(data_content) = content {
                        let entry = DataEntry {
                            content: data_content,
                            query_info: query_info.clone(),
                        };
                        
                        // Check if this is a new key
                        if !self.data.contains_key(&key) {
                            // Check if it's not already assigned to any panel or unassigned list
                            let is_assigned = self.config.panels.iter().any(|p| p.contains(&key))
                                || self.config.unassigned.contains(&key);
                            
                            if !is_assigned {
                                self.config.unassigned.push(key.clone());
                                self.save_config();
                            }
                        }
                        
                        self.data.insert(key, entry);
                    }
                }
            }
        }

        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Apollos Kiosk");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!("{} data feeds", self.data.len()));
                });
            });
        });

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
                            ui.label(egui::RichText::new("📦 Unassigned Feeds").heading().strong());
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
    fn render_panel(&mut self, ui: &mut egui::Ui, panel_idx: usize) {
        let panel_names = ["Left Panel", "Center Panel", "Right Panel"];
        
        egui::Frame::group(ui.style())
            .fill(ui.visuals().panel_fill)
            .rounding(8.0)
            .inner_margin(8.0)
            .show(ui, |ui| {
                ui.set_min_height(ui.available_height());
                
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(panel_names[panel_idx]).strong().size(16.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(egui::RichText::new(format!("{}", self.config.panels[panel_idx].len())).weak().small());
                    });
                });
                ui.separator();
                ui.add_space(4.0);

                let keys = self.config.panels[panel_idx].clone();
                let mut to_remove = None;
                let mut to_move = None;

                for (idx, key) in keys.iter().enumerate() {
                    if let Some(entry) = self.data.get(key) {
                        self.render_large_card(ui, key, content, panel_idx, idx, &mut to_remove, &mut to_move);
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
    ) {
        let card_frame = egui::Frame::group(ui.style())
            .fill(ui.visuals().faint_bg_color)
            .stroke(egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color))
            .rounding(12.0)
            .inner_margin(16.0)
            .outer_margin(egui::Margin::symmetric(0, 8));

        card_frame.show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            
            // Card header with title and controls
            ui.horizontal(|ui| {
                // Use query name if available, otherwise fall back to key
                let display_name = entry.query_info
                    .as_ref()
                    .map(|q| q.name.as_str())
                    .unwrap_or(key);
                    
                ui.label(egui::RichText::new(display_name).heading().strong().size(18.0));
                
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(key).weak().small());
                    ui.add_space(8.0);
                    ui.menu_button("⋮", |ui| {
                        if ui.button("🗑 Unassign").clicked() {
                            *to_remove = Some(card_idx);
                            ui.close_menu();
                        }
                        ui.separator();
                        ui.label("Move to:");
                        for (idx, name) in ["Left", "Center", "Right"].iter().enumerate() {
                            if idx != panel_idx {
                                if ui.button(format!("➜ {}", name)).clicked() {
                                    *to_move = Some((card_idx, idx));
                                    ui.close_menu();
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
                CondensedData::Gtfs(routes) => self.render_gtfs_card(ui, routes),
                CondensedData::Gbfs(stations) => self.render_gbfs_card(ui, stations),
                CondensedData::Weather(reports) => self.render_weather_card(ui, reports),
                CondensedData::Calendar(events) => self.render_calendar_card(ui, events),
                CondensedData::Aqi(reports) => self.render_aqi_card(ui, reports),
                CondensedData::Tidal(reports) => self.render_tidal_card(ui, reports),
                _ => {
                    ui.label(egui::RichText::new("Data type not yet supported in card view").weak());
                }
            }
        });
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

    fn render_gtfs_card(&self, ui: &mut egui::Ui, routes: &[GtfsCondensed]) {
        for r in routes {
            egui::Frame::none()
                .fill(ui.visuals().extreme_bg_color)
                .rounding(8.0)
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        // Mode icon
                        ui.label(egui::RichText::new(Self::mode_icon(&r.mode)).size(28.0));
                        ui.add_space(8.0);
                        
                        // Route number/name
                        ui.label(egui::RichText::new(&r.route).heading().strong().size(24.0));
                        ui.add_space(8.0);
                        
                        // Destination info
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new(&r.dest).size(18.0));
                            ui.label(egui::RichText::new(&r.dir).size(13.0).color(ui.visuals().weak_text_color()));
                        });
                    });
                    
                    ui.add_space(8.0);
                    
                    // Determine which times to show (prioritize live times)
                    let times_to_show: Vec<(String, bool)> = if let Some(live_times) = &r.times_live {
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
                        ui.label(egui::RichText::new("Next arrivals:").strong().size(14.0));
                        ui.add_space(4.0);
                        
                        ui.horizontal_wrapped(|ui| {
                            for (time, is_live) in times_to_show {
                                ui.horizontal(|ui| {
                                    // Icon to indicate live vs scheduled
                                    let (icon, color) = if is_live {
                                        (ICON_RADIO, egui::Color32::from_rgb(76, 175, 80)) // Green for live
                                    } else {
                                        (ICON_SCHEDULE, ui.visuals().weak_text_color()) // Gray for scheduled
                                    };
                                    
                                    ui.label(egui::RichText::new(icon).size(14.0).color(color));
                                    ui.label(
                                        egui::RichText::new(&time)
                                            .monospace()
                                            .size(16.0)
                                            .color(ui.visuals().strong_text_color())
                                            .background_color(ui.visuals().code_bg_color)
                                    );
                                });
                            }
                        });
                        }
                });
            ui.add_space(8.0);
        }
    }

    fn render_gbfs_card(&self, ui: &mut egui::Ui, stations: &[GbfsCondensed]) {
        for s in stations {
            egui::Frame::none()
                .fill(ui.visuals().extreme_bg_color)
                .rounding(8.0)
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.label(egui::RichText::new(&s.name).strong().size(16.0));
                    ui.add_space(8.0);
                    
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(format!("🚲 {} available", s.avail)).size(18.0));
                        ui.separator();
                        ui.label(egui::RichText::new(format!("⚡ {}", s.avail_elec)).size(18.0));
                        ui.separator();
                        ui.label(egui::RichText::new(format!("🅿 {} docks", s.docks_avail)).size(18.0));
                    });
                });
            ui.add_space(8.0);
        }
    }

    fn render_weather_card(&self, ui: &mut egui::Ui, reports: &[WeatherCondensed]) {
        for w in reports {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("{:.0}°", w.temp)).size(56.0).strong());
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new(&w.weather).strong().size(22.0));
                    ui.label(egui::RichText::new(format!("Feels like {:.0}°", w.feel)).size(16.0));
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(format!("💨 {}mph  💧 {}%", w.wind.speed, w.hum)).size(14.0).weak());
                });
            });
        }
    }

    fn render_calendar_card(&self, ui: &mut egui::Ui, events: &[CalendarCondensed]) {
        for e in events {
            egui::Frame::none()
                .fill(ui.visuals().extreme_bg_color)
                .rounding(8.0)
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.label(egui::RichText::new(&e.description).strong().size(16.0));
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(&e.date_start).monospace().size(14.0).weak());
                });
            ui.add_space(8.0);
        }
    }

    fn render_aqi_card(&self, ui: &mut egui::Ui, reports: &[AqiCondensed]) {
        for a in reports {
            ui.label(egui::RichText::new(&a.name).strong().size(16.0));
            ui.add_space(4.0);
            ui.label(egui::RichText::new(format!("📊 {} measurements", a.measurements.len())).size(14.0));
            ui.add_space(8.0);
        }
    }

    fn render_tidal_card(&self, ui: &mut egui::Ui, reports: &[TidalCondensed]) {
        for t in reports {
            if let Some(h) = &t.first_high { 
                ui.label(egui::RichText::new(format!("⬆ High: {}", h)).size(16.0));
                ui.add_space(4.0);
            }
            if let Some(l) = &t.first_low { 
                ui.label(egui::RichText::new(format!("⬇ Low: {}", l)).size(16.0));
                ui.add_space(4.0);
            }
        }
    }
}

fn main() -> eframe::Result {
        let args = Args::parse();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1920.0, 1080.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Apollos Kiosk",
        options,
        Box::new(|cc| Ok(Box::new(ApollosKiosk::new(cc, args)))),
    )
}

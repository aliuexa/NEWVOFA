use crossbeam::channel::Receiver;
use eframe::egui;
use egui::Color32;
use egui_plot::{Line, Plot, PlotPoints};
use newvofa_buffer::DataStore;
use newvofa_serial::{SerialConfig, SerialManager, SerialMessage};
use std::collections::BTreeMap;
use std::sync::Arc;
use parking_lot::Mutex;

const CHANNEL_COLORS: [Color32; 16] = [
    Color32::from_rgb(0, 255, 0),
    Color32::from_rgb(255, 0, 0),
    Color32::from_rgb(0, 128, 255),
    Color32::from_rgb(255, 255, 0),
    Color32::from_rgb(255, 0, 255),
    Color32::from_rgb(0, 255, 255),
    Color32::from_rgb(255, 128, 0),
    Color32::from_rgb(128, 0, 255),
    Color32::from_rgb(0, 255, 128),
    Color32::from_rgb(255, 128, 128),
    Color32::from_rgb(128, 255, 0),
    Color32::from_rgb(0, 128, 128),
    Color32::from_rgb(128, 0, 128),
    Color32::from_rgb(128, 128, 255),
    Color32::from_rgb(255, 200, 100),
    Color32::from_rgb(100, 200, 255),
];

const FUNC_COLORS: [Color32; 8] = [
    Color32::from_rgb(255, 255, 255),
    Color32::from_rgb(255, 160, 60),
    Color32::from_rgb(200, 120, 255),
    Color32::from_rgb(60, 255, 160),
    Color32::from_rgb(255, 220, 40),
    Color32::from_rgb(100, 255, 255),
    Color32::from_rgb(255, 100, 200),
    Color32::from_rgb(160, 255, 60),
];

fn extract_var_names(expr: &str, aliases: &[String]) -> Vec<String> {
    let builtins: &[&str] = &[
        "sin", "cos", "tan", "asin", "acos", "atan", "atan2",
        "sinh", "cosh", "tanh", "asinh", "acosh", "atanh",
        "sqrt", "abs", "exp", "ln", "log2", "log10",
        "floor", "ceil", "round", "signum",
        "max", "min", "pi", "e",
    ];
    let chars: Vec<char> = expr.chars().collect();
    let mut names: Vec<String> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_alphabetic() || chars[i] == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            if !builtins.contains(&word.as_str()) && aliases.iter().any(|a| !a.is_empty() && a.as_str() == word.as_str()) {
                if !names.contains(&word) {
                    names.push(word);
                }
            }
        } else {
            i += 1;
        }
    }
    names
}

struct FuncEntry {
    expression: String,
    show: bool,
    error: Option<String>,
    var_names: Vec<String>,
}

impl FuncEntry {
    fn new() -> Self {
        Self {
            expression: String::new(),
            show: true,
            error: None,
            var_names: Vec::new(),
        }
    }

    fn parse(&mut self, aliases: &[String]) {
        self.error = None;
        self.var_names.clear();

        let trimmed = self.expression.trim();
        if trimmed.is_empty() {
            return;
        }

        let expr_result: Result<meval::Expr, _> = trimmed.parse();
        match expr_result {
            Ok(_expr) => {
                self.var_names = extract_var_names(trimmed, aliases);
            }
            Err(e) => {
                self.error = Some(format!("Parse: {}", e));
            }
        }
    }

    fn eval_point(&self, aliases: &[String], all_channels: &[&newvofa_buffer::RingBuffer<f32>], sample_idx: usize) -> Option<f64> {
        if self.expression.trim().is_empty() || self.var_names.is_empty() {
            return None;
        }
        let trimmed = self.expression.trim();
        let expr: meval::Expr = trimmed.parse().ok()?;
        let mut ctx = meval::Context::new();
        for name in &self.var_names {
            if let Some(ch_idx) = aliases.iter().position(|a| !a.is_empty() && a == name.as_str()) {
                if let Some(ch) = all_channels.get(ch_idx) {
                    if let Some(&v) = ch.get(sample_idx) {
                        ctx.var(name.clone(), v as f64);
                    } else {
                        return None;
                    }
                } else {
                    return None;
                }
            } else {
                return None;
            }
        }
        let result = expr.eval_with_context(ctx).ok()?;
        if result.is_finite() {
            Some(result)
        } else {
            None
        }
    }
}

impl Clone for FuncEntry {
    fn clone(&self) -> Self {
        Self {
            expression: self.expression.clone(),
            show: self.show,
            error: self.error.clone(),
            var_names: self.var_names.clone(),
        }
    }
}

struct VofaApp {
    serial_manager: Option<SerialManager>,
    frame_receiver: Option<Receiver<SerialMessage>>,
    available_ports: Vec<newvofa_serial::PortInfo>,
    selected_port: String,
    baud_rate: u32,
    is_connected: bool,
    status_message: String,

    data_store: Arc<Mutex<DataStore>>,
    num_channels: usize,
    max_points: usize,

    auto_detect: bool,
    paused: bool,
    show_channel: Vec<bool>,
    follow_data: bool,
    visible_points: usize,

    command_name: String,
    command_value: String,
    raw_hex_buffer: String,
    raw_hex_paused: bool,
    ch_drag_start: Option<egui::Pos2>,
    ch_drag_rect: Option<egui::Rect>,
    ch_drag_target: bool,
    row_rects: Vec<(usize, egui::Rect)>,

    channel_aliases: Vec<String>,
    func_entries: Vec<FuncEntry>,

    parameters: BTreeMap<String, f32>,
    param_edits: BTreeMap<String, String>,
}

impl VofaApp {
    fn new() -> Self {
        let num_channels = 8;
        let max_points = 2000;
        Self {
            serial_manager: None,
            frame_receiver: None,
            available_ports: Vec::new(),
            selected_port: String::new(),
            baud_rate: 115200,
            is_connected: false,
            status_message: String::from("Ready"),
            data_store: Arc::new(Mutex::new(DataStore::new(num_channels, max_points))),
            num_channels,
            max_points,
            auto_detect: true,
            paused: false,
            show_channel: vec![true; num_channels],
            follow_data: true,
            visible_points: 500,
            command_name: String::new(),
            command_value: String::new(),
            raw_hex_buffer: String::with_capacity(65536),
            raw_hex_paused: false,
            ch_drag_start: None,
            ch_drag_rect: None,
            ch_drag_target: false,
            row_rects: Vec::new(),
            channel_aliases: vec![String::new(); num_channels],
            func_entries: Vec::new(),
            parameters: BTreeMap::new(),
            param_edits: BTreeMap::new(),
        }
    }

    fn refresh_ports(&mut self) {
        self.available_ports = SerialManager::list_ports();
    }

    fn connect(&mut self) {
        if self.selected_port.is_empty() {
            self.status_message = "Please select a port".into();
            return;
        }

        let config = SerialConfig {
            port_name: self.selected_port.clone(),
            baud_rate: self.baud_rate,
            ..Default::default()
        };

        match SerialManager::open(config) {
            Ok((manager, receiver)) => {
                self.serial_manager = Some(manager);
                self.frame_receiver = Some(receiver);
                self.is_connected = true;
                self.status_message = format!("Connected to {}", self.selected_port);
            }
            Err(e) => {
                self.status_message = format!("Connect failed: {}", e);
            }
        }
    }

    fn disconnect(&mut self) {
        self.serial_manager = None;
        self.frame_receiver = None;
        self.is_connected = false;
        self.raw_hex_buffer.clear();
        self.status_message = "Disconnected".into();
    }

    fn poll_frames(&mut self) {
        let mut raw_chunks: Vec<Vec<u8>> = Vec::new();
        if let Some(ref receiver) = self.frame_receiver {
            let mut store = self.data_store.lock();
            while let Ok(msg) = receiver.try_recv() {
                match msg {
                    SerialMessage::Frame(frame) => {
                        if self.auto_detect && frame.len() != self.num_channels {
                            self.num_channels = frame.len();
                            self.show_channel = vec![true; frame.len()];
                            self.channel_aliases.resize(frame.len(), String::new());
                            store.resize_channels(frame.len(), self.max_points);
                        }
                        if !self.paused {
                            store.push_frame(&frame);
                        }
                    }
                    SerialMessage::RawData(raw) => {
                        if !self.raw_hex_paused {
                            raw_chunks.push(raw);
                        }
                    }
                    SerialMessage::Parameter(entries) => {
                        for entry in entries {
                            self.parameters.insert(entry.name.clone(), entry.value);
                            self.param_edits.entry(entry.name.clone()).or_insert_with(|| entry.value.to_string());
                        }
                    }
                }
            }
        }
        for raw in raw_chunks {
            self.append_hex_data(&raw);
        }
    }

    fn append_hex_data(&mut self, data: &[u8]) {
        const MAX_HEX_LEN: usize = 65536;
        for &byte in data {
            if !self.raw_hex_buffer.is_empty() {
                self.raw_hex_buffer.push(' ');
            }
            use std::fmt::Write;
            let _ = write!(self.raw_hex_buffer, "{:02X}", byte);
        }
        if self.raw_hex_buffer.len() > MAX_HEX_LEN {
            let excess = self.raw_hex_buffer.len() - MAX_HEX_LEN;
            let keep_from = self.raw_hex_buffer
                .char_indices()
                .nth(excess)
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.raw_hex_buffer = self.raw_hex_buffer[keep_from..].to_string();
        }
    }

    fn send_command(&mut self) {
        if self.command_name.is_empty() {
            return;
        }
        let value: f32 = match self.command_value.parse() {
            Ok(v) => v,
            Err(_) => {
                self.status_message = "Invalid command value".into();
                return;
            }
        };
        if let Some(ref mut manager) = self.serial_manager {
            match manager.send_command(&self.command_name, value) {
                Ok(()) => {
                    self.status_message = format!("Sent: {}:{}", self.command_name, value);
                }
                Err(e) => {
                    self.status_message = e;
                }
            }
        }
    }

    fn apply_channel_count(&mut self) {
        let count = self.num_channels.max(1).min(100);
        self.num_channels = count;
        self.show_channel = vec![true; count];
        self.channel_aliases.resize(count, String::new());
        self.data_store
            .lock()
            .resize_channels(count, self.max_points);
    }

    fn post_frame_drag(&mut self, ctx: &egui::Context) {
        let pointer_down = ctx.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
        let pointer_pos = ctx.pointer_hover_pos();

        if !pointer_down {
            self.ch_drag_start = None;
            self.ch_drag_rect = None;
            return;
        }

        if let Some(pos) = pointer_pos {
            if self.ch_drag_start.is_none() {
                for (i, row_rect) in &self.row_rects {
                    if row_rect.contains(pos) {
                        self.ch_drag_start = Some(pos);
                        if *i < self.show_channel.len() {
                            self.ch_drag_target = self.show_channel[*i];
                        }
                        break;
                    }
                }
            }

            if let Some(start) = self.ch_drag_start {
                let rect = egui::Rect::from_two_pos(start, pos);
                self.ch_drag_rect = Some(rect);

                for (i, row_rect) in &self.row_rects {
                    if rect.intersects(*row_rect) {
                        if *i < self.show_channel.len() {
                            self.show_channel[*i] = self.ch_drag_target;
                        }
                    }
                }
            }
        }
    }
}

impl eframe::App for VofaApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_frames();

        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Gmaster Serial Viewer - Serial Plot");
                ui.separator();
                ui.label(&self.status_message);
            });
        });

        egui::SidePanel::left("control_panel")
            .resizable(true)
            .default_width(220.0)
            .show(ctx, |ui| {
                self.render_control_panel(ui);
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            self.render_plot(ui);
        });

        egui::TopBottomPanel::bottom("hex_panel")
            .resizable(true)
            .default_height(200.0)
            .show(ctx, |ui| {
                self.render_hex_panel(ui);
            });

        self.post_frame_drag(ctx);

        if let Some(rect) = self.ch_drag_rect {
            ctx.debug_painter().rect_filled(
                rect,
                egui::Rounding::ZERO,
                Color32::from_rgba_premultiplied(51, 153, 255, 60),
            );
            ctx.debug_painter().rect_stroke(
                rect,
                egui::Rounding::ZERO,
                egui::Stroke::new(1.0, Color32::from_rgb(51, 153, 255)),
            );
        }

        ctx.request_repaint();
    }
}

impl VofaApp {
    fn render_control_panel(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
        ui.heading("Serial Port");

        ui.horizontal(|ui| {
            if ui.button("Refresh").clicked() {
                self.refresh_ports();
            }
        });

        egui::ComboBox::from_label("Port")
            .selected_text(&self.selected_port)
            .show_ui(ui, |ui| {
                for port in &self.available_ports {
                    let label = format!("{} - {}", port.name, port.description);
                    ui.selectable_value(&mut self.selected_port, port.name.clone(), label);
                }
            });

        ui.horizontal(|ui| {
            ui.label("Baud Rate:");
            egui::ComboBox::from_id_salt("baud_rate")
                .selected_text(format!("{}", self.baud_rate))
                .show_ui(ui, |ui| {
                    for &rate in &[9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600] {
                        ui.selectable_value(&mut self.baud_rate, rate, format!("{}", rate));
                    }
                });
        });

        ui.add_space(8.0);

        if self.is_connected {
            if ui
                .add_sized([ui.available_width(), 30.0], egui::Button::new("Disconnect"))
                .clicked()
            {
                self.disconnect();
            }
        } else {
            if ui
                .add_sized([ui.available_width(), 30.0], egui::Button::new("Connect"))
                .clicked()
            {
                self.connect();
            }
        }

        ui.add_space(16.0);
        ui.heading("Channels");

        ui.checkbox(&mut self.auto_detect, "Auto-detect from frame");

        ui.horizontal(|ui| {
            ui.label("Count:");
            if self.auto_detect {
                ui.label(format!("{} (auto)", self.num_channels));
            } else {
                let mut count = self.num_channels;
                if ui
                    .add(egui::DragValue::new(&mut count).range(1..=100))
                    .changed()
                {
                    self.num_channels = count;
                    self.apply_channel_count();
                }
            }
        });

        ui.horizontal(|ui| {
            ui.label("History:");
            let mut pts = self.max_points;
            if ui
                .add(egui::DragValue::new(&mut pts).range(100..=50000))
                .changed()
            {
                self.max_points = pts;
                self.apply_channel_count();
            }
        });

        ui.add_space(8.0);

        if ui
            .button(if self.paused { "Resume" } else { "Pause" })
            .clicked()
        {
            self.paused = !self.paused;
        }

        if ui.button("Clear").clicked() {
            self.data_store.lock().clear();
        }

        ui.add_space(4.0);

        ui.checkbox(&mut self.follow_data, "Auto-follow");

        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.label("Window:");
            ui.add(
                egui::Slider::new(&mut self.visible_points, 50..=5000)
                    .text("pts"),
            );
        });

        ui.add_space(8.0);

        let old_selectable = ui.style().interaction.selectable_labels;
        ui.style_mut().interaction.selectable_labels = false;

        let mut row_rects: Vec<(usize, egui::Rect)> = Vec::new();

        for i in 0..self.num_channels {
            let color = CHANNEL_COLORS[i % CHANNEL_COLORS.len()];
            let row = ui.horizontal(|ui| {
                ui.colored_label(color, format!("CH{}", i + 1));
                ui.set_min_width(36.0);
                if self.show_channel.len() > i {
                    ui.add(egui::Checkbox::without_text(
                        &mut self.show_channel[i],
                    ));
                    if self.channel_aliases.len() > i {
                        ui.add_sized(
                            [50.0, 18.0],
                            egui::TextEdit::singleline(&mut self.channel_aliases[i])
                                .hint_text("alias"),
                        );
                    }
                }
            });
            row_rects.push((i, row.response.rect));
        }

        ui.style_mut().interaction.selectable_labels = old_selectable;
        self.row_rects = row_rects;

        ui.add_space(16.0);
        ui.heading("Command");

        ui.horizontal(|ui| {
            ui.label("Name:");
            ui.text_edit_singleline(&mut self.command_name);
        });

        ui.horizontal(|ui| {
            ui.label("Value:");
            ui.text_edit_singleline(&mut self.command_value);
        });

        if ui
            .add_sized(
                [ui.available_width(), 24.0],
                egui::Button::new("Send Command"),
            )
            .clicked()
        {
            self.send_command();
        }

        ui.add_space(16.0);
        self.render_param_ui(ui);
        ui.add_space(16.0);
        self.render_func_ui(ui);
        });
    }

    fn send_param_update(&mut self, name: &str) {
        let val_str = match self.param_edits.get(name) {
            Some(s) => s.clone(),
            None => return,
        };
        let value: f32 = match val_str.parse() {
            Ok(v) => v,
            Err(_) => {
                self.status_message = format!("Invalid value for {}", name);
                return;
            }
        };
        if let Some(ref mut manager) = self.serial_manager {
            match manager.send_command(name, value) {
                Ok(()) => {
                    self.status_message = format!("Sent: {}:{}", name, value);
                }
                Err(e) => {
                    self.status_message = e;
                }
            }
        }
    }

    fn render_param_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Parameters");

        if self.parameters.is_empty() {
            ui.label("(waiting for parameter data...)");
            return;
        }

        for (name, value) in self.parameters.clone().iter() {
            ui.horizontal(|ui| {
                ui.label(name);
                ui.colored_label(Color32::from_rgb(100, 200, 255), format!("{:.3}", value));

                let edit = self.param_edits.entry(name.clone()).or_insert_with(|| value.to_string());
                ui.add_sized(
                    [80.0, 20.0],
                    egui::TextEdit::singleline(edit).hint_text("new value"),
                );

                if ui.small_button("Set").clicked() {
                    self.send_param_update(name);
                }
            });
        }
    }

    fn render_func_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Functions");

        let aliases = self.channel_aliases.clone();
        for entry in &mut self.func_entries {
            entry.parse(&aliases);
        }

        ui.horizontal(|ui| {
            if ui.button("+ Add").clicked() {
                self.func_entries.push(FuncEntry::new());
            }
            if ui.button("Clear All").clicked() {
                self.func_entries.clear();
            }
        });

        let mut remove_idx: Option<usize> = None;
        let entries = &mut self.func_entries;

        for (i, entry) in entries.iter_mut().enumerate() {
            let color = FUNC_COLORS[i % FUNC_COLORS.len()];

            ui.horizontal(|ui| {
                let mut show = entry.show;
                if ui.add(egui::Checkbox::new(&mut show, "")).changed() {
                    entry.show = show;
                }

                let text_id = egui::Id::new(("func_expr", i));
                let mut expr = entry.expression.clone();
                let text_response = ui.add_sized(
                    [ui.available_width() - 30.0, 20.0],
                    egui::TextEdit::singleline(&mut expr)
                        .id(text_id)
                        .hint_text("e.g. a*sin(b)+c"),
                );
                if text_response.changed() && expr != entry.expression {
                    entry.expression = expr;
                }

                if ui
                    .add_sized([20.0, 20.0], egui::Button::new("✕"))
                    .clicked()
                {
                    remove_idx = Some(i);
                }
            });

            if let Some(ref err) = entry.error {
                ui.colored_label(Color32::from_rgb(255, 80, 80), err);
            } else if !entry.expression.trim().is_empty() && entry.var_names.is_empty() {
                ui.colored_label(
                    Color32::from_rgb(160, 160, 160),
                    "(no matching aliases)",
                );
            } else if !entry.expression.trim().is_empty() {
                let vars: Vec<String> = entry
                    .var_names
                    .iter()
                    .map(|name| {
                        let idx = self.channel_aliases.iter().position(|a| a == name.as_str());
                        match idx {
                            Some(ch) => format!("{}->CH{}", name, ch + 1),
                            None => format!("{}->?", name),
                        }
                    })
                    .collect();
                ui.colored_label(
                    color,
                    format!("  binds: {}", vars.join(", ")),
                );
            }
        }

        if let Some(idx) = remove_idx {
            self.func_entries.remove(idx);
        }
    }

    fn render_plot(&mut self, ui: &mut egui::Ui) {
        let store = self.data_store.lock();
        let sample_count = store.sample_count();
        let follow = self.follow_data;
        let window = self.visible_points as u64;

        let plot_id = if follow {
            "vofa_plot_follow"
        } else {
            "vofa_plot_free"
        };

        let mut y_min: Option<f64> = None;
        let mut y_max: Option<f64> = None;
        let mut all_lines: Vec<(Vec<[f64; 2]>, Color32, String)> = Vec::new();

        for i in 0..self.num_channels {
            if i < self.show_channel.len() && !self.show_channel[i] {
                continue;
            }

            if let Some(channel) = store.channel_data(i) {
                if channel.is_empty() {
                    continue;
                }

                let color = CHANNEL_COLORS[i % CHANNEL_COLORS.len()];

                let (visible_len, start_idx) = if follow {
                    let len = (channel.len() as u64).min(window) as usize;
                    let start = sample_count.saturating_sub(len as u64);
                    (len, start)
                } else {
                    let len = channel.len();
                    let start = sample_count.saturating_sub(len as u64);
                    (len, start)
                };

                let skip = channel.len().saturating_sub(visible_len);
                let mut points_vec: Vec<[f64; 2]> =
                    Vec::with_capacity(visible_len);
                for (j, val_ref) in channel.iter().skip(skip).enumerate() {
                    let val: f64 = (*val_ref) as f64;
                    if !val.is_finite() {
                        continue;
                    }
                    let x: f64 = (start_idx + j as u64) as f64;
                    points_vec.push([x, val]);

                    y_min = Some(y_min.map_or(val, |v| v.min(val)));
                    y_max = Some(y_max.map_or(val, |v| v.max(val)));
                }

                if i == 0 && !points_vec.is_empty() {
                    let last = points_vec.last().unwrap();
                    log::info!(
                        "CH1 latest: x={:.3}, y={:.6}, window={}",
                        last[0], last[1], visible_len
                    );
                }

                if !points_vec.is_empty() {
                    all_lines.push((points_vec, color, format!("CH{}", i + 1)));
                }
            }
        }

        if !self.func_entries.is_empty() {
            let channel_refs: Vec<&newvofa_buffer::RingBuffer<f32>> = (0..self.num_channels)
                .filter_map(|i| store.channel_data(i))
                .collect();

            let min_len = channel_refs.iter().map(|c| c.len()).min().unwrap_or(0);
            let vis_len = if follow {
                (min_len as u64).min(window) as usize
            } else {
                min_len
            };
            let vis_start = sample_count.saturating_sub(vis_len as u64);
            let skip = min_len.saturating_sub(vis_len);

            for (fi, entry) in self.func_entries.iter().enumerate() {
                if !entry.show || entry.expression.trim().is_empty() || entry.var_names.is_empty() {
                    continue;
                }
                if vis_len == 0 {
                    continue;
                }

                let color = FUNC_COLORS[fi % FUNC_COLORS.len()];
                let mut points = Vec::with_capacity(vis_len);

                for j in 0..vis_len {
                    let idx = skip + j;
                    if let Some(result) = entry.eval_point(&self.channel_aliases, &channel_refs, idx) {
                        let x = (vis_start + j as u64) as f64;
                        points.push([x, result]);
                        y_min = Some(y_min.map_or(result, |v| v.min(result)));
                        y_max = Some(y_max.map_or(result, |v| v.max(result)));
                    }
                }

                if !points.is_empty() {
                    let label = format!("f{}: {}", fi + 1, entry.expression);
                    all_lines.push((points, color, label));
                }
            }
        }

        if let (Some(lo), Some(hi)) = (y_min, y_max) {
            let span = hi - lo;
            if span.abs() < 1e-9 {
                y_min = Some(lo - 1.0);
                y_max = Some(hi + 1.0);
            } else {
                let pad = span * 0.05;
                y_min = Some(lo - pad);
                y_max = Some(hi + pad);
            }
        }

        let mut plot = Plot::new(plot_id)
            .x_axis_label("Sample")
            .y_axis_label("Value")
            .allow_drag(!follow)
            .allow_zoom(!follow)
            .allow_scroll(!follow);

        if let (Some(lo), Some(hi)) = (y_min, y_max) {
            plot = plot.include_y(lo);
            plot = plot.include_y(hi);
        }

        plot.show(ui, |plot_ui| {
            for (points, color, name) in &all_lines {
                plot_ui.line(
                    Line::new(PlotPoints::new(points.clone()))
                        .color(*color)
                        .name(name.clone()),
                );
            }
        });
    }

    fn render_hex_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("HEX Raw Data");
            ui.separator();
            if ui
                .button(if self.raw_hex_paused { "Resume HEX" } else { "Pause HEX" })
                .clicked()
            {
                self.raw_hex_paused = !self.raw_hex_paused;
            }
            if ui.button("Clear HEX").clicked() {
                self.raw_hex_buffer.clear();
            }
            ui.label(format!("{} bytes", self.raw_hex_buffer.len()));
        });

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let text = if self.raw_hex_buffer.is_empty() {
                    "(waiting for data...)"
                } else {
                    &self.raw_hex_buffer
                };
                let mut display_text = text.to_string();
                ui.add(
                    egui::TextEdit::multiline(&mut display_text)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(f32::INFINITY)
                        .interactive(false),
                );
            });
    }
}

fn main() -> Result<(), eframe::Error> {
    env_logger::init();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 720.0])
            .with_title("Gmaster Serial Viewer"),
        ..Default::default()
    };

    eframe::run_native(
        "Gmaster Serial Viewer",
        options,
        Box::new(|_cc| Ok(Box::new(VofaApp::new()))),
    )
}

// ============================================================
// DeepSeek 小鲸鱼余额挂件 - 便携版
// 所有配置保存在 exe 同目录下的 config.json
// ============================================================

use eframe::{egui, Frame, NativeOptions};
use egui::{Align2, Color32, Context, FontId, Vec2, Window};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

// ==================== 便携配置路径 ====================

/// 获取 exe 所在目录
fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// 获取配置文件路径（exe 同目录）
fn config_path() -> PathBuf {
    exe_dir().join("config.json")
}

/// 获取账本文件路径（exe 同目录）
fn ledger_path() -> PathBuf {
    exe_dir().join("ledger.json")
}

// ==================== 数据模型 ====================

#[derive(Serialize, Deserialize, Clone)]
struct Config {
    api_key: String,
    widget_size: f32,
    usage_mode: UsageMode,
    auto_refresh_seconds: u64,
    show_bubble: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            widget_size: 1.0,
            usage_mode: UsageMode::Ledger,
            auto_refresh_seconds: 60,
            show_bubble: true,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq)]
enum UsageMode {
    Ledger,
    Realtime,
}

#[derive(Serialize, Deserialize, Clone)]
struct UsageRecord {
    date: String,
    amount: f64,
}

// ==================== 核心应用 ====================

struct WhaleApp {
    // 数据
    balance: f64,
    currency: String,
    today_usage: f64,
    config: Config,
    ledger: Vec<UsageRecord>,
    
    // UI 状态
    show_menu: bool,
    show_bubble: bool,
    bubble_text: String,
    bubble_timer: Option<Instant>,
    last_error: Option<String>,
    
    // 动画
    animated_balance: f64,
    target_balance: f64,
    anim_start: Option<Instant>,
    anim_start_value: f64,
    
    // 网络
    client: reqwest::Client,
    balance_receiver: Option<mpsc::Receiver<f64>>,
    is_loading: bool,
    last_refresh: Instant,
}

impl Default for WhaleApp {
    fn default() -> Self {
        // 确保配置目录存在（exe 同目录）
        let config_file = config_path();
        let ledger_file = ledger_path();
        
        // 加载配置
        let config: Config = if config_file.exists() {
            fs::read_to_string(&config_file)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default()
        } else {
            let default = Config::default();
            // 创建默认配置
            if let Ok(json) = serde_json::to_string_pretty(&default) {
                let _ = fs::write(&config_file, json);
            }
            default
        };
        
        // 加载账本
        let ledger: Vec<UsageRecord> = if ledger_file.exists() {
            fs::read_to_string(&ledger_file)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        
        Self {
            balance: 0.0,
            currency: "CNY".to_string(),
            today_usage: 0.0,
            config,
            ledger,
            show_menu: false,
            show_bubble: false,
            bubble_text: "你好！🐋".to_string(),
            bubble_timer: None,
            last_error: None,
            animated_balance: 0.0,
            target_balance: 0.0,
            anim_start: None,
            anim_start_value: 0.0,
            client: reqwest::Client::new(),
            balance_receiver: None,
            is_loading: false,
            last_refresh: Instant::now(),
        }
    }
}

impl WhaleApp {
    // ==================== 配置管理 ====================
    
    fn save_config(&self) {
        let path = config_path();
        if let Ok(json) = serde_json::to_string_pretty(&self.config) {
            let _ = fs::write(&path, json);
        }
    }
    
    fn save_ledger(&self) {
        let path = ledger_path();
        if let Ok(json) = serde_json::to_string_pretty(&self.ledger) {
            let _ = fs::write(&path, json);
        }
    }
    
    // ==================== 获取余额 ====================
    
    fn fetch_balance(&mut self) {
        if self.config.api_key.is_empty() {
            self.last_error = Some("❌ 请设置 API Key".to_string());
            return;
        }
        
        if self.is_loading {
            return;
        }
        
        self.is_loading = true;
        let (tx, rx) = mpsc::channel();
        self.balance_receiver = Some(rx);
        
        let client = self.client.clone();
        let api_key = self.config.api_key.clone();
        
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let url = "https://api.deepseek.com/user/balance";
                let result = client
                    .get(url)
                    .header("Authorization", format!("Bearer {}", api_key))
                    .send()
                    .await;
                
                if let Ok(resp) = result {
                    if let Ok(json) = resp.json::<serde_json::Value>().await {
                        if let Some(total) = json.get("totalBalance").and_then(|v| v.as_f64()) {
                            let _ = tx.send(total);
                        }
                    }
                }
            });
        });
    }
    
    // ==================== 记账逻辑 ====================
    
    fn update_ledger(&mut self, new_balance: f64) {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        
        // 检查币种是否变化
        if new_balance < self.balance {
            let diff = self.balance - new_balance;
            
            if let Some(record) = self.ledger.iter_mut().find(|r| r.date == today) {
                record.amount += diff;
            } else {
                self.ledger.push(UsageRecord {
                    date: today.clone(),
                    amount: diff,
                });
            }
            
            self.today_usage = self.ledger
                .iter()
                .find(|r| r.date == today)
                .map(|r| r.amount)
                .unwrap_or(0.0);
            
            // 清理 30 天前的记录
            let cutoff = chrono::Local::now().naive_local().date() - chrono::Days::new(30);
            self.ledger.retain(|r| {
                chrono::NaiveDate::parse_from_str(&r.date, "%Y-%m-%d")
                    .map(|d| d >= cutoff)
                    .unwrap_or(false)
            });
            
            self.save_ledger();
        }
        
        self.balance = new_balance;
    }
    
    // ==================== 气泡 ====================
    
    fn show_bubble(&mut self, text: &str) {
        self.bubble_text = text.to_string();
        self.show_bubble = true;
        self.bubble_timer = Some(Instant::now());
    }
    
    fn random_speech(&mut self) -> &'static str {
        use rand::seq::SliceRandom;
        let speeches = [
            "你好呀！🐋",
            "今天也要加油！💪",
            "摸摸头～ ✨",
            "咕噜咕噜～ 🌊",
            "余额还够吗？💰",
            "深海宝藏等你发现！🗺️",
            "小鲸鱼永远陪着你！❤️",
            "今天的你也很棒！🌟",
        ];
        speeches.choose(&mut rand::thread_rng()).unwrap_or(&"你好！")
    }
}

// ==================== UI 实现 ====================

impl eframe::App for WhaleApp {
    fn update(&mut self, ctx: &Context, _frame: &mut Frame) {
        // ---- 定时刷新 ----
        if self.last_refresh.elapsed() > Duration::from_secs(self.config.auto_refresh_seconds) {
            self.fetch_balance();
            self.last_refresh = Instant::now();
        }
        
        // ---- 处理异步数据 ----
        if let Some(receiver) = &self.balance_receiver {
            if let Ok(new_balance) = receiver.try_recv() {
                self.target_balance = new_balance;
                self.anim_start = Some(Instant::now());
                self.anim_start_value = self.animated_balance;
                
                if matches!(self.config.usage_mode, UsageMode::Ledger) {
                    self.update_ledger(new_balance);
                }
                self.is_loading = false;
                self.last_error = None;
            }
        }
        
        // ---- 动画更新 ----
        if let Some(start) = self.anim_start {
            let elapsed = start.elapsed().as_secs_f32();
            if elapsed < 0.8 {
                let progress = elapsed / 0.8;
                let eased = 1.0 - (1.0 - progress).powi(3);
                self.animated_balance = self.anim_start_value 
                    + (self.target_balance - self.anim_start_value) * eased;
                ctx.request_repaint_after(Duration::from_millis(16));
            } else {
                self.animated_balance = self.target_balance;
                self.anim_start = None;
            }
        }
        
        // ---- 气泡超时 ----
        if self.show_bubble {
            if let Some(timer) = self.bubble_timer {
                if timer.elapsed() > Duration::from_secs(4) {
                    self.show_bubble = false;
                }
            }
        }
        
        // ==================== 主界面 ====================
        
        let size = 180.0 * self.config.widget_size;
        
        egui::CentralPanel::default()
            .frame(egui::Frame {
                fill: Color32::TRANSPARENT,
                ..Default::default()
            })
            .show(ctx, |ui| {
                ui.set_min_size(Vec2::new(size, size * 1.3));
                let rect = ui.available_rect_before_wrap();
                let painter = ui.painter();
                
                // ---- 鲸鱼身体 ----
                let center = rect.center();
                let radius = size * 0.35;
                
                // 身体渐变
                painter.circle_filled(center, radius, Color32::from_rgb(70, 130, 200));
                painter.circle_filled(
                    center + Vec2::new(0.0, radius * 0.3),
                    radius * 0.6,
                    Color32::from_rgb(100, 160, 230),
                );
                
                // 肚子亮色
                painter.circle_filled(
                    center + Vec2::new(0.0, radius * 0.4),
                    radius * 0.45,
                    Color32::from_rgb(160, 210, 245),
                );
                
                // ---- 眼睛 ----
                let eye_y = center.y - radius * 0.1;
                painter.circle_filled(
                    center + Vec2::new(-radius * 0.25, eye_y - center.y),
                    radius * 0.12,
                    Color32::WHITE,
                );
                painter.circle_filled(
                    center + Vec2::new(radius * 0.25, eye_y - center.y),
                    radius * 0.12,
                    Color32::WHITE,
                );
                painter.circle_filled(
                    center + Vec2::new(-radius * 0.22, eye_y - center.y),
                    radius * 0.06,
                    Color32::BLACK,
                );
                painter.circle_filled(
                    center + Vec2::new(radius * 0.28, eye_y - center.y),
                    radius * 0.06,
                    Color32::BLACK,
                );
                
                // ---- 微笑 ----
                painter.line_segment(
                    [
                        center + Vec2::new(-radius * 0.2, radius * 0.12),
                        center + Vec2::new(radius * 0.2, radius * 0.12),
                    ],
                    egui::Stroke::new(radius * 0.04, Color32::BLACK),
                );
                
                // ---- 喷泉 ----
                for i in 0..7 {
                    let x = center.x + (i as f32 - 3.0) * radius * 0.07;
                    let y = center.y - radius * 0.7 - (i as f32 - 3.0).powi(2).abs() * radius * 0.005;
                    let alpha = 200 - (i as f32 - 3.0).abs() * 30;
                    painter.circle_filled(
                        egui::Pos2::new(x, y),
                        radius * 0.025,
                        Color32::from_rgb(150, 200, 255),
                    );
                }
                
                // ---- 余额 ----
                let balance_text = format!("💰 {:.2}", self.animated_balance);
                let font_size = size * 0.12;
                let text_pos = egui::Pos2::new(
                    rect.left() + size * 0.05,
                    rect.top() + size * 0.05,
                );
                
                // 阴影
                painter.text(
                    text_pos + Vec2::new(1.0, 1.0),
                    Align2::LEFT_TOP,
                    &balance_text,
                    FontId::proportional(font_size),
                    Color32::from_rgba_premultiplied(0, 0, 0, 100),
                );
                painter.text(
                    text_pos,
                    Align2::LEFT_TOP,
                    &balance_text,
                    FontId::proportional(font_size),
                    Color32::WHITE,
                );
                
                // ---- 今日使用 ----
                if self.today_usage > 0.0 {
                    let usage_text = format!("📊 {:.2}", self.today_usage);
                    painter.text(
                        egui::Pos2::new(rect.left() + size * 0.05, rect.top() + size * 0.22),
                        Align2::LEFT_TOP,
                        &usage_text,
                        FontId::proportional(size * 0.07),
                        Color32::from_rgb(200, 220, 255),
                    );
                }
                
                // ---- 加载中 ----
                if self.is_loading {
                    painter.text(
                        rect.right_bottom() + Vec2::new(-size * 0.05, -size * 0.05),
                        Align2::RIGHT_BOTTOM,
                        "⏳",
                        FontId::proportional(size * 0.08),
                        Color32::WHITE,
                    );
                }
                
                // ---- 错误 ----
                if let Some(error) = &self.last_error {
                    painter.text(
                        egui::Pos2::new(rect.left() + size * 0.05, rect.bottom() - size * 0.08),
                        Align2::LEFT_BOTTOM,
                        error,
                        FontId::proportional(size * 0.06),
                        Color32::from_rgb(255, 100, 100),
                    );
                }
                
                // ---- 气泡 ----
                if self.show_bubble && self.config.show_bubble {
                    let bubble_rect = egui::Rect::from_min_size(
                        egui::Pos2::new(rect.right() - size * 0.7, rect.top() + size * 0.05),
                        Vec2::new(size * 0.6, size * 0.2),
                    );
                    
                    painter.rect_filled(
                        bubble_rect,
                        8.0,
                        Color32::from_rgba_premultiplied(255, 255, 255, 220),
                    );
                    
                    painter.path_filled(
                        vec![
                            bubble_rect.left_bottom(),
                            bubble_rect.left_bottom() + Vec2::new(-8.0, 0.0),
                            bubble_rect.left_bottom() + Vec2::new(0.0, 8.0),
                        ],
                        Color32::from_rgba_premultiplied(255, 255, 255, 220),
                    );
                    
                    painter.text(
                        bubble_rect.center(),
                        Align2::CENTER_CENTER,
                        &self.bubble_text,
                        FontId::proportional(size * 0.09),
                        Color32::BLACK,
                    );
                }
                
                // ---- 交互 ----
                let response = ui.interact(
                    rect,
                    egui::Id::new("whale"),
                    egui::Sense::click_and_drag(),
                );
                
                // 左键点击
                if response.clicked() {
                    let speech = self.random_speech();
                    self.show_bubble(speech);
                    self.fetch_balance();
                }
                
                // 右键菜单
                if response.secondary_clicked() {
                    self.show_menu = !self.show_menu;
                }
                
                // 拖拽
                if response.dragged() {
                    let delta = response.drag_delta();
                    if let Some(viewport) = ctx.input(|i| i.viewport().clone()) {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Move(
                            viewport.position.unwrap_or_default() + delta
                        ));
                    }
                }
            });
        
        // ==================== 菜单 ====================
        
        if self.show_menu {
            Window::new("🐋 设置")
                .default_pos(egui::Pos2::new(50.0, 50.0))
                .default_size(Vec2::new(300.0, 400.0))
                .resizable(false)
                .show(ctx, |ui| {
                    ui.heading("🐋 小鲸鱼设置");
                    ui.separator();
                    
                    ui.label("🔑 DeepSeek API Key:");
                    if ui.text_edit_singleline(&mut self.config.api_key).changed() {
                        self.save_config();
                    }
                    
                    ui.label("📏 大小:");
                    if ui.add(egui::Slider::new(&mut self.config.widget_size, 0.6..=2.0))
                        .changed() 
                    {
                        self.save_config();
                    }
                    
                    ui.label("⏱️ 刷新间隔 (秒):");
                    if ui.add(egui::Slider::new(&mut self.config.auto_refresh_seconds, 10..=300))
                        .changed() 
                    {
                        self.save_config();
                    }
                    
                    ui.horizontal(|ui| {
                        ui.label("📊 模式:");
                        if ui.selectable_label(
                            matches!(self.config.usage_mode, UsageMode::Ledger),
                            "📒 记账"
                        ) {
                            self.config.usage_mode = UsageMode::Ledger;
                            self.save_config();
                        }
                        if ui.selectable_label(
                            matches!(self.config.usage_mode, UsageMode::Realtime),
                            "🔄 实时"
                        ) {
                            self.config.usage_mode = UsageMode::Realtime;
                            self.save_config();
                        }
                    });
                    
                    ui.checkbox(&mut self.config.show_bubble, "💬 显示气泡");
                    if ui.checkbox(&mut self.config.show_bubble, "").changed() {
                        self.save_config();
                    }
                    
                    ui.separator();
                    
                    ui.horizontal(|ui| {
                        if ui.button("🔄 刷新余额").clicked() {
                            self.fetch_balance();
                        }
                        if ui.button("💾 保存配置").clicked() {
                            self.save_config();
                        }
                    });
                    
                    ui.separator();
                    
                    ui.label(format!("💰 余额: {:.2} {}", self.balance, self.currency));
                    ui.label(format!("📈 今日使用: {:.2}", self.today_usage));
                    
                    if let Some(error) = &self.last_error {
                        ui.colored_label(Color32::RED, error);
                    }
                    
                    ui.separator();
                    ui.label(format!("📁 配置位置: {}", config_path().display()));
                    ui.label("💡 右键点击鲸鱼打开菜单");
                    ui.label("💡 左键点击刷新余额");
                });
        }
        
        // ---- 持续刷新 ----
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}

// ==================== 主函数 ====================

fn main() -> Result<(), eframe::Error> {
    let options = NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([200.0, 260.0])
            .with_min_inner_size([150.0, 180.0])
            .with_max_inner_size([400.0, 520.0])
            .with_resizable(false)
            .with_transparent(true)
            .with_decorations(false)
            .with_always_on_top()
            .with_skip_taskbar(true)
            .with_title("🐋 DeepSeek 小鲸鱼"),
        ..Default::default()
    };
    
    eframe::run_native(
        "🐋 DeepSeek 小鲸鱼",
        options,
        Box::new(|_cc| Box::new(WhaleApp::default())),
    )
}

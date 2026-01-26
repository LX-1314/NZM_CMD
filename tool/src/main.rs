#![windows_subsystem = "windows"]

use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use screenshots::Screen;
use std::fs;
use std::time::Instant;

// ==========================================
// 1. 数据结构 (Data Structures)
// ==========================================
#[derive(Clone, PartialEq)]
enum RecognitionLogic { AND, OR }

#[derive(Clone, PartialEq)]
enum ElementKind {
    TextAnchor { text: String },
    ColorAnchor { color_hex: String, tolerance: u8 },
    Button { target: String, post_delay: u32 },
}

#[derive(Clone)]
struct UIElementDraft {
    pos_or_rect: Rect,
    kind: ElementKind,
}

// ==========================================
// 2. 编辑器核心状态 (App State)
// ==========================================
struct MapBuilderTool {
    texture: Option<egui::TextureHandle>,
    raw_image: Option<image::RgbaImage>, 
    img_size: Vec2,
    scene_id: String,
    scene_name: String,
    logic: RecognitionLogic,
    
    start_pos: Option<Pos2>,
    current_rect: Option<Rect>,
    is_color_picker_mode: bool,
    
    capture_timer: Option<Instant>, 

    drafts: Vec<UIElementDraft>,
    toml_output: String,
}

impl MapBuilderTool {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup_custom_fonts(&cc.egui_ctx); // 加载微软雅黑

        Self {
            texture: None,
            raw_image: None,
            img_size: Vec2::ZERO,
            scene_id: "lobby_01".into(),
            scene_name: "游戏主界面".into(),
            logic: RecognitionLogic::AND,
            start_pos: None,
            current_rect: None,
            is_color_picker_mode: false,
            capture_timer: None,
            drafts: Vec::new(),
            toml_output: String::new(),
        }
    }

    fn capture_immediate(&mut self, ctx: &egui::Context) {
        let screens = Screen::all().unwrap();
        if let Some(screen) = screens.first() {
            if let Ok(image) = screen.capture() {
                self.img_size = Vec2::new(image.width() as f32, image.height() as f32);
                self.raw_image = Some(image.clone()); 
                let color_img = egui::ColorImage::from_rgba_unmultiplied(
                    [image.width() as usize, image.height() as usize], 
                    image.as_flat_samples().as_slice()
                );
                self.texture = Some(ctx.load_texture("shot", color_img, Default::default()));
            }
        }
    }

    fn pick_color(&self, p: Pos2) -> String {
        if let Some(img) = &self.raw_image {
            let x = p.x as u32;
            let y = p.y as u32;
            if x < img.width() && y < img.height() {
                let pixel = img.get_pixel(x, y);
                return format!("#{:02X}{:02X}{:02X}", pixel[0], pixel[1], pixel[2]);
            }
        }
        "#FFFFFF".into()
    }

    fn build_toml(&mut self) {
        let logic_str = if self.logic == RecognitionLogic::AND { "and" } else { "or" };
        let mut toml = format!("[[scenes]]\nid = \"{}\"\nname = \"{}\"\nlogic = \"{}\"\n\n", 
                                self.scene_id, self.scene_name, logic_str);
        
        toml.push_str("# --- 识别特征 ---\n");
        for d in &self.drafts {
            match &d.kind {
                ElementKind::TextAnchor { text } => {
                    toml.push_str(&format!("anchors.text = {{ rect = [{}, {}, {}, {}], val = \"{}\" }}\n",
                        d.pos_or_rect.min.x as i32, d.pos_or_rect.min.y as i32, d.pos_or_rect.max.x as i32, d.pos_or_rect.max.y as i32, text));
                }
                ElementKind::ColorAnchor { color_hex, tolerance } => {
                    toml.push_str(&format!("anchors.color = {{ pos = [{}, {}], val = \"{}\", tol = {} }}\n",
                        d.pos_or_rect.min.x as i32, d.pos_or_rect.min.y as i32, color_hex, tolerance));
                }
                _ => {}
            }
        }

        toml.push_str("\n# --- 跳转动作 ---\n");
        for d in &self.drafts {
            if let ElementKind::Button { target, post_delay } = &d.kind {
                toml.push_str("[[scenes.transitions]]\n");
                toml.push_str(&format!("target = \"{}\"\n", target));
                toml.push_str(&format!("coords = [{}, {}]\n", d.pos_or_rect.center().x as i32, d.pos_or_rect.center().y as i32));
                toml.push_str(&format!("post_delay = {}\n\n", post_delay));
            }
        }
        self.toml_output = toml;
    }
}

// ==========================================
// 3. 字体加载配置 (解决中文乱码)
// ==========================================
fn setup_custom_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    if let Ok(data) = fs::read("C:\\Windows\\Fonts\\msyh.ttc") {
        fonts.font_data.insert("msyh".to_owned(), egui::FontData::from_owned(data));
        fonts.families.get_mut(&egui::FontFamily::Proportional).unwrap().insert(0, "msyh".to_owned());
        fonts.families.get_mut(&egui::FontFamily::Monospace).unwrap().insert(0, "msyh".to_owned());
    }
    ctx.set_fonts(fonts);
}

// ==========================================
// 4. GUI 渲染与交互 (包含 ID 修复)
// ==========================================
impl eframe::App for MapBuilderTool {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(start_time) = self.capture_timer {
            let elapsed = start_time.elapsed().as_secs_f32();
            if elapsed >= 3.0 {
                self.capture_immediate(ctx);
                self.capture_timer = None; 
                self.drafts.clear(); 
                self.current_rect = None;
            } else {
                ctx.request_repaint(); 
            }
        }

        egui::SidePanel::left("side").min_width(350.0).show(ctx, |ui| {
            ui.heading("🚀 MINKE UI 自动化建模器");
            ui.add_space(10.0);
            
            ui.group(|ui| {
                if self.capture_timer.is_some() {
                    let remaining = 3.0 - self.capture_timer.unwrap().elapsed().as_secs_f32();
                    ui.add(egui::ProgressBar::new(remaining / 3.0)
                        .text(format!("倒计时识别：{:.1}秒", remaining)));
                } else {
                    if ui.button("📸 3秒延时截图").clicked() {
                        self.capture_timer = Some(Instant::now());
                    }
                }
            });

            ui.separator();
            ui.horizontal(|ui| { ui.label("场景ID:"); ui.text_edit_singleline(&mut self.scene_id); });
            ui.horizontal(|ui| { ui.label("名称:"); ui.text_edit_singleline(&mut self.scene_name); });
            ui.horizontal(|ui| { 
                ui.label("场景判定:"); 
                ui.radio_value(&mut self.logic, RecognitionLogic::AND, "AND"); 
                ui.radio_value(&mut self.logic, RecognitionLogic::OR, "OR"); 
            });

            ui.separator();
            ui.checkbox(&mut self.is_color_picker_mode, "开启取色模式 (吸管)");

            if let Some(rect) = self.current_rect {
                ui.group(|ui| {
                    // 颜色优化：将原先的金黄色改为青色 (Cyan)，对比度更高
                    // ui.label(RichText::new("已选中目标：").color(Color32::CYAN).strong());
                    ui.label(RichText::new("已选中目标：").color(Color32::from_rgb(0, 255, 255)).strong());
                    if self.is_color_picker_mode {
                        let color = self.pick_color(rect.min);
                        ui.label(format!("像素颜色: {}", color));
                        if ui.button("添加为颜色锚点").clicked() {
                            self.drafts.push(UIElementDraft { pos_or_rect: rect, kind: ElementKind::ColorAnchor { color_hex: color, tolerance: 15 } });
                            self.current_rect = None;
                        }
                    } else {
                        if ui.button("添加为 OCR 锚点").clicked() {
                            self.drafts.push(UIElementDraft { pos_or_rect: rect, kind: ElementKind::TextAnchor { text: "输入文本".into() } });
                            self.current_rect = None;
                        }
                        if ui.button("添加为跳转按钮").clicked() {
                            self.drafts.push(UIElementDraft { pos_or_rect: rect, kind: ElementKind::Button { target: "next_id".into(), post_delay: 500 } });
                            self.current_rect = None;
                        }
                    }
                });
            }

            ui.separator();
            ui.label("元素池:");
            // 修复点：通过 id_source 显式指定 ID，解决界面上的红色警告
            egui::ScrollArea::vertical().id_source("list_scroll").max_height(250.0).show(ui, |ui| {
                let mut del = None;
                for (i, d) in self.drafts.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        match &mut d.kind {
                            ElementKind::TextAnchor { text } => { ui.label("⚓"); ui.text_edit_singleline(text); }
                            ElementKind::ColorAnchor { color_hex, tolerance } => {
                                ui.label("🧪"); ui.label(color_hex.as_str());
                                ui.add(egui::DragValue::new(tolerance).clamp_range(0..=100).prefix("T:"));
                            }
                            ElementKind::Button { target, post_delay } => {
                                ui.label("🖱️"); ui.text_edit_singleline(target);
                                ui.add(egui::DragValue::new(post_delay).speed(10).prefix("ms:"));
                            }
                        }
                        if ui.button("❌").clicked() { del = Some(i); }
                    });
                }
                if let Some(i) = del { self.drafts.remove(i); }
            });

            ui.separator();
            if ui.button("💾 生成 TOML").clicked() { self.build_toml(); }
            // 修复点：第二个滚动区域也需要唯一的 ID
            egui::ScrollArea::vertical().id_source("toml_scroll").show(ui, |ui| {
                ui.add(egui::TextEdit::multiline(&mut self.toml_output)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY));
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::drag());
            if let Some(tex) = &self.texture {
                let painter_size = resp.rect.size();
                let scale = (painter_size.x / self.img_size.x).min(painter_size.y / self.img_size.y);
                let draw_size = self.img_size * scale;
                let draw_rect = Rect::from_min_size(resp.rect.min, draw_size);
                painter.image(tex.id(), draw_rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);

                let to_screen = |p: Pos2| draw_rect.min + (p.to_vec2() * scale);
                let from_screen = |p: Pos2| {
                    let v = (p - draw_rect.min) / scale;
                    Pos2::new(v.x, v.y)
                };

                for d in &self.drafts {
                    let color = match d.kind {
                        ElementKind::TextAnchor{..} => Color32::GREEN,
                        ElementKind::ColorAnchor{..} => Color32::from_rgb(255, 165, 0),
                        ElementKind::Button{..} => Color32::BLUE,
                    };
                    painter.rect_stroke(Rect::from_min_max(to_screen(d.pos_or_rect.min), to_screen(d.pos_or_rect.max)), 2.0, Stroke::new(2.0, color));
                }

                if resp.drag_started() {
                    if let Some(p) = resp.interact_pointer_pos() { self.start_pos = Some(from_screen(p)); }
                }
                if let (Some(start), Some(curr_raw)) = (self.start_pos, resp.interact_pointer_pos()) {
                    let curr = from_screen(curr_raw);
                    let rect = if self.is_color_picker_mode {
                        Rect::from_min_max(curr, curr + Vec2::splat(1.0))
                    } else {
                        Rect::from_two_pos(start, curr)
                    };
                    painter.rect_stroke(Rect::from_min_max(to_screen(rect.min), to_screen(rect.max)), 0.0, Stroke::new(1.5, Color32::RED));
                    if resp.drag_released() { self.current_rect = Some(rect); self.start_pos = None; }
                }
            } else {
                ui.centered_and_justified(|ui| ui.label("点击左侧『3秒延时截图』开始建模"));
            }
        });
    }
}

fn main() -> eframe::Result<()> {
    let opts = eframe::NativeOptions { viewport: egui::ViewportBuilder::default().with_inner_size([1400.0, 900.0]), ..Default::default() };
    eframe::run_native("MINKE UI Mapper Pro", opts, Box::new(|cc| Box::new(MapBuilderTool::new(cc))))
}
#![windows_subsystem = "windows"]

use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use screenshots::Screen;
use serde::Deserialize;
use std::fs;
use std::time::Instant;

// OCR 所需的引用
use std::io::Cursor;
use windows::Media::Ocr::{OcrEngine, OcrResult}; 
use windows::Graphics::Imaging::BitmapDecoder;
use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};

// ==========================================
// 1. 数据结构
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

#[derive(Deserialize)]
struct TomlRoot { scenes: Vec<TomlScene> }
#[derive(Deserialize)]
struct TomlScene { id: String, name: String, logic: String, anchors: Option<TomlAnchors>, transitions: Option<Vec<TomlTransition>> }
#[derive(Deserialize)]
struct TomlAnchors { text: Option<Vec<TomlTextAnchor>>, color: Option<Vec<TomlColorAnchor>> }
#[derive(Deserialize)]
struct TomlTextAnchor { rect: [i32; 4], val: String }
#[derive(Deserialize)]
struct TomlColorAnchor { pos: [i32; 2], val: String, tol: u8 }
#[derive(Deserialize)]
struct TomlTransition { target: String, coords: [i32; 2], post_delay: u32 }

// ==========================================
// 2. 编辑器状态
// ==========================================
struct MapBuilderTool {
    texture: Option<egui::TextureHandle>,
    raw_image: Option<image::RgbaImage>, 
    img_size: Vec2,
    
    ocr_engine: Option<OcrEngine>,
    ocr_test_result: String, 

    scene_id: String,
    scene_name: String,
    logic: RecognitionLogic,
    
    start_pos: Option<Pos2>,
    current_rect: Option<Rect>,
    is_color_picker_mode: bool,
    capture_timer: Option<Instant>, 

    drafts: Vec<UIElementDraft>,
    toml_content: String,
    status_msg: String,
}

unsafe impl Send for MapBuilderTool {}

impl MapBuilderTool {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup_custom_fonts(&cc.egui_ctx);
        
        let engine = OcrEngine::TryCreateFromUserProfileLanguages().ok();
        let status = if engine.is_some() { "OCR 引擎就绪" } else { "⚠️ OCR 初始化失败" };

        Self {
            texture: None,
            raw_image: None,
            img_size: Vec2::ZERO,
            ocr_engine: engine,          
            ocr_test_result: String::new(), 
            scene_id: "lobby_01".into(),
            scene_name: "游戏主界面".into(),
            logic: RecognitionLogic::AND,
            start_pos: None,
            current_rect: None,
            is_color_picker_mode: false,
            capture_timer: None,
            drafts: Vec::new(),
            toml_content: String::new(),
            status_msg: status.into(),
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
                self.status_msg = "截图成功".into();
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
        let mut toml = format!("[[scenes]]\nid = \"{}\"\nname = \"{}\"\nlogic = \"{}\"\n\n", self.scene_id, self.scene_name, logic_str);
        toml.push_str("[scenes.anchors]\n");
        toml.push_str("text = [\n");
        for d in self.drafts.iter() {
            if let ElementKind::TextAnchor { text } = &d.kind {
                toml.push_str(&format!("  {{ rect = [{}, {}, {}, {}], val = \"{}\" }},\n",
                    d.pos_or_rect.min.x as i32, d.pos_or_rect.min.y as i32, d.pos_or_rect.max.x as i32, d.pos_or_rect.max.y as i32, text));
            }
        }
        toml.push_str("]\ncolor = [\n");
        for d in self.drafts.iter() {
            if let ElementKind::ColorAnchor { color_hex, tolerance } = &d.kind {
                toml.push_str(&format!("  {{ pos = [{}, {}], val = \"{}\", tol = {} }},\n",
                    d.pos_or_rect.min.x as i32, d.pos_or_rect.min.y as i32, color_hex, tolerance));
            }
        }
        toml.push_str("]\n\n# --- 动作步骤 ---\n");
        for d in self.drafts.iter() {
            if let ElementKind::Button { target, post_delay } = &d.kind {
                toml.push_str("[[scenes.transitions]]\n");
                toml.push_str(&format!("target = \"{}\"\n", target));
                toml.push_str(&format!("coords = [{}, {}]\n", d.pos_or_rect.center().x as i32, d.pos_or_rect.center().y as i32));
                toml.push_str(&format!("post_delay = {}\n\n", post_delay));
            }
        }
        self.toml_content = toml;
        self.status_msg = "TOML 已生成".into();
    }

    fn import_toml(&mut self) {
        if self.toml_content.trim().is_empty() { self.status_msg = "导入失败：内容为空".into(); return; }
        match toml::from_str::<TomlRoot>(&self.toml_content) {
            Ok(root) => {
                if let Some(scene) = root.scenes.first() {
                    self.scene_id = scene.id.clone();
                    self.scene_name = scene.name.clone();
                    self.logic = if scene.logic.to_lowercase() == "or" { RecognitionLogic::OR } else { RecognitionLogic::AND };
                    self.drafts.clear();
                    if let Some(anchors) = &scene.anchors {
                        if let Some(texts) = &anchors.text {
                            for t in texts {
                                let rect = Rect::from_min_max(Pos2::new(t.rect[0] as f32, t.rect[1] as f32), Pos2::new(t.rect[2] as f32, t.rect[3] as f32));
                                self.drafts.push(UIElementDraft { pos_or_rect: rect, kind: ElementKind::TextAnchor { text: t.val.clone() } });
                            }
                        }
                        if let Some(colors) = &anchors.color {
                            for c in colors {
                                let pos = Pos2::new(c.pos[0] as f32, c.pos[1] as f32);
                                let rect = Rect::from_min_max(pos, pos + Vec2::splat(1.0));
                                self.drafts.push(UIElementDraft { pos_or_rect: rect, kind: ElementKind::ColorAnchor { color_hex: c.val.clone(), tolerance: c.tol } });
                            }
                        }
                    }
                    if let Some(transitions) = &scene.transitions {
                        for t in transitions {
                            let rect = Rect::from_center_size(Pos2::new(t.coords[0] as f32, t.coords[1] as f32), Vec2::splat(20.0));
                            self.drafts.push(UIElementDraft { pos_or_rect: rect, kind: ElementKind::Button { target: t.target.clone(), post_delay: t.post_delay } });
                        }
                    }
                    self.status_msg = format!("成功导入场景：{}", self.scene_id);
                }
            },
            Err(e) => { self.status_msg = format!("解析失败: {}", e); }
        }
    }

    fn perform_ocr(&mut self, rect: Rect) {
        if self.ocr_engine.is_none() {
            self.ocr_test_result = "OCR 引擎未初始化".into();
            return;
        }
        if let Some(img) = &self.raw_image {
            let x = rect.min.x.max(0.0) as u32;
            let y = rect.min.y.max(0.0) as u32;
            let w = rect.width().max(1.0) as u32;
            let h = rect.height().max(1.0) as u32;

            if x + w > img.width() || y + h > img.height() {
                self.ocr_test_result = "区域超出图片范围".into();
                return;
            }

            let sub_img = image::imageops::crop_imm(img, x, y, w, h).to_image();
            let scaled_img = image::imageops::resize(&sub_img, w * 2, h * 2, image::imageops::FilterType::Lanczos3);
            let dynamic_img = image::DynamicImage::ImageRgba8(scaled_img);

            let mut png_buffer = Cursor::new(Vec::new());
            if dynamic_img.write_to(&mut png_buffer, image::ImageFormat::Png).is_err() {
                self.ocr_test_result = "图像编码失败".into();
                return;
            }
            
            self.ocr_test_result = "识别中...".into();
            let engine = self.ocr_engine.as_ref().unwrap();
            let png_bytes = png_buffer.into_inner();

            let run_recognition = || -> windows::core::Result<String> {
                let stream = InMemoryRandomAccessStream::new()?;
                let writer = DataWriter::CreateDataWriter(&stream)?;
                writer.WriteBytes(&png_bytes)?;
                writer.StoreAsync()?.get()?;
                writer.FlushAsync()?.get()?;
                stream.Seek(0)?;

                let decoder = BitmapDecoder::CreateAsync(&stream)?.get()?;
                let bmp = decoder.GetSoftwareBitmapAsync()?.get()?;
                let result: OcrResult = engine.RecognizeAsync(&bmp)?.get()?;
                
                let mut text = String::new();
                if let Ok(lines) = result.Lines() {
                    for line in lines {
                        if let Ok(h_str) = line.Text() {
                            text.push_str(&h_str.to_string());
                        }
                    }
                }
                Ok(text.replace(char::is_whitespace, ""))
            };

            match run_recognition() {
                Ok(txt) => {
                    self.ocr_test_result = if txt.is_empty() { "无文字".to_string() } else { txt };
                    self.status_msg = format!("OCR 完成: {}", self.ocr_test_result);
                },
                Err(e) => {
                    self.ocr_test_result = format!("API 错误: {:?}", e);
                }
            }
        }
    }
} // 🔥 MapBuilderTool 实现块结束

// ==========================================
// 3. UI 实现
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

impl eframe::App for MapBuilderTool {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(start_time) = self.capture_timer {
            if start_time.elapsed().as_secs_f32() >= 3.0 {
                self.capture_immediate(ctx);
                self.capture_timer = None; 
                self.drafts.clear(); 
                self.current_rect = None;
            } else {
                ctx.request_repaint(); 
            }
        }

        egui::SidePanel::left("side").min_width(350.0).show(ctx, |ui| {
            ui.heading("🚀 MINKE UI 建模器 (OCR测试)");
            ui.label(RichText::new(&self.status_msg).color(Color32::from_rgb(0, 255, 128))); 
            ui.add_space(5.0);
            
            ui.group(|ui| {
                if self.capture_timer.is_some() {
                    let remaining = 3.0 - self.capture_timer.unwrap().elapsed().as_secs_f32();
                    ui.add(egui::ProgressBar::new(remaining / 3.0).text(format!("倒计时：{:.1}s", remaining)));
                } else {
                    if ui.button("📸 3秒延时截图").clicked() { self.capture_timer = Some(Instant::now()); }
                }
            });

            ui.separator();
            ui.horizontal(|ui| { ui.label("ID:"); ui.text_edit_singleline(&mut self.scene_id); });
            ui.horizontal(|ui| { ui.label("名称:"); ui.text_edit_singleline(&mut self.scene_name); });
            ui.horizontal(|ui| { 
                ui.label("逻辑:"); 
                ui.radio_value(&mut self.logic, RecognitionLogic::AND, "AND"); 
                ui.radio_value(&mut self.logic, RecognitionLogic::OR, "OR"); 
            });

            ui.separator();
            ui.checkbox(&mut self.is_color_picker_mode, "🧪 吸管取色模式");

            if let Some(rect) = self.current_rect {
                ui.group(|ui| {
                    ui.label(RichText::new("已选中目标：").color(Color32::from_rgb(0, 255, 255)).strong());
                    
                    if self.is_color_picker_mode {
                        let color = self.pick_color(rect.min);
                        ui.label(format!("HEX: {}", color));
                        if ui.button("📌 添加颜色锚点").clicked() {
                            self.drafts.push(UIElementDraft { pos_or_rect: rect, kind: ElementKind::ColorAnchor { color_hex: color, tolerance: 15 } });
                            self.current_rect = None;
                        }
                    } else {
                        ui.horizontal(|ui| {
                            if ui.button("⚓ 添加 Text 锚点").clicked() {
                                let val = if self.ocr_test_result.is_empty() || self.ocr_test_result.contains("...") { "Text".to_string() } else { self.ocr_test_result.clone() };
                                self.drafts.push(UIElementDraft { pos_or_rect: rect, kind: ElementKind::TextAnchor { text: val } });
                                self.current_rect = None;
                            }
                            if ui.button("🔍 区域 OCR 测试").clicked() {
                                self.perform_ocr(rect);
                            }
                        });
                        
                        if !self.ocr_test_result.is_empty() {
                            ui.label(RichText::new(format!("识别结果: [{}]", self.ocr_test_result)).color(Color32::BLACK));
                        }

                        if ui.button("🖱️ 添加 Button 跳转").clicked() {
                            self.drafts.push(UIElementDraft { pos_or_rect: rect, kind: ElementKind::Button { target: "next".into(), post_delay: 500 } });
                            self.current_rect = None;
                        }
                    }
                });
            }

            ui.separator();
            egui::ScrollArea::vertical().id_source("list_scroll").max_height(200.0).show(ui, |ui| {
                let mut del = None;
                for (i, d) in self.drafts.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        match &mut d.kind {
                            ElementKind::TextAnchor { text } => { ui.label("⚓"); ui.text_edit_singleline(text); }
                            ElementKind::ColorAnchor { color_hex, tolerance } => {
                                ui.label("🧪"); ui.label(color_hex.as_str());
                                ui.add(egui::DragValue::new(tolerance).prefix("T:"));
                            }
                            ElementKind::Button { target, post_delay } => {
                                ui.label("🖱️"); ui.text_edit_singleline(target);
                                ui.add(egui::DragValue::new(post_delay).prefix("ms:"));
                            }
                        }
                        if ui.button("❌").clicked() { del = Some(i); }
                    });
                }
                if let Some(i) = del { self.drafts.remove(i); }
            });

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("📤 生成 TOML").clicked() { self.build_toml(); }
                if ui.button("📥 导入 TOML").clicked() { self.import_toml(); }
            });
            
            egui::ScrollArea::vertical().id_source("toml_scroll").show(ui, |ui| {
                ui.add(egui::TextEdit::multiline(&mut self.toml_content).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY));
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
                let from_screen = |p: Pos2| { let v = (p - draw_rect.min) / scale; Pos2::new(v.x, v.y) };

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
                    let rect = if self.is_color_picker_mode { Rect::from_min_max(curr, curr + Vec2::splat(1.0)) } else { Rect::from_two_pos(start, curr) };
                    painter.rect_stroke(Rect::from_min_max(to_screen(rect.min), to_screen(rect.max)), 0.0, Stroke::new(1.5, Color32::RED));
                    if resp.drag_released() { 
                        self.current_rect = Some(rect); 
                        self.start_pos = None; 
                        self.ocr_test_result.clear(); 
                    }
                }
            } else {
                ui.centered_and_justified(|ui| ui.label("点击左侧『3秒延时截图』开始工作"));
            }
        });
    }
}

fn main() -> eframe::Result<()> {
    let opts = eframe::NativeOptions { viewport: egui::ViewportBuilder::default().with_inner_size([1400.0, 900.0]), ..Default::default() };
    eframe::run_native("MINKE UI Mapper Pro", opts, Box::new(|cc| Box::new(MapBuilderTool::new(cc))))
}
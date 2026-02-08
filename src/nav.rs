// src/nav.rs
use crate::human::HumanDriver;
use serde::Deserialize;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::sync::atomic::AtomicUsize;
use std::thread;
use std::time::{Duration, Instant};
use std::fs;
use std::path::Path;
use std::io::Cursor;

use screenshots::Screen;
use windows::Media::Ocr::OcrEngine;
use windows::Globalization::Language;
use windows::Graphics::Imaging::BitmapDecoder;
use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};

// ==========================================
// 0. 结果枚举
// ==========================================
#[derive(Debug, PartialEq)]
pub enum NavResult {
    Success,
    // ✨ 修改：Handover 携带 (场景ID, 处理器代号)
    Handover(String, Option<String>),
    Failed,
}

// ==========================================
// 1. TOML 配置数据结构
// ==========================================
#[derive(Deserialize, Debug, Clone)]
struct TomlRoot { scenes: Vec<Scene> }

#[derive(Deserialize, Debug, Clone)]
struct Scene {
    id: String,
    #[serde(default)] logic: String,
    #[serde(default)] anchors: Option<Anchors>,
    #[serde(default)] transitions: Option<Vec<Transition>>,
    // ✨ 新增：处理该界面的函数代号 (例如 "daily", "td")
    #[serde(default)]
    handler: Option<String>,
}

#[derive(Deserialize, Debug, Clone, Default)]
struct Anchors {
    text: Option<Vec<TextAnchor>>,
    color: Option<Vec<ColorAnchor>>,
}

#[derive(Deserialize, Debug, Clone)]
struct TextAnchor {
    rect: [i32; 4],
    val: String,
}

#[derive(Deserialize, Debug, Clone)]
struct ColorAnchor {
    pos: [i32; 2],
    val: String,
    tol: u8,
}

#[derive(Deserialize, Debug, Clone)]
struct Transition {
    target: String,
    coords: [i32; 2],
    #[serde(default = "default_delay")]
    post_delay: u64,
}

fn default_delay() -> u64 { 500 }

// ==========================================
// 2. 接口层 (OCR 与 多重图像预处理)
// ==========================================
struct GameInterface {
    driver: Arc<Mutex<HumanDriver>>,
    ocr_engine: Option<OcrEngine>,
    screenshot_count: AtomicUsize, 
}

unsafe impl Send for GameInterface {}
unsafe impl Sync for GameInterface {}

impl GameInterface {
    fn new(driver: Arc<Mutex<HumanDriver>>) -> Self {
        println!("🚀 初始化 Windows OCR...");
        let engine = match Language::CreateLanguage(&windows::core::HSTRING::from("zh-Hans")) {
            Ok(lang) => match OcrEngine::TryCreateFromLanguage(&lang) {
                Ok(e) => Some(e),
                Err(_) => OcrEngine::TryCreateFromUserProfileLanguages().ok()
            },
            Err(_) => OcrEngine::TryCreateFromUserProfileLanguages().ok(),
        };
        Self { 
            driver, 
            ocr_engine: engine,
            screenshot_count: AtomicUsize::new(0), 
        }
    }

    /// 调用底层 Windows OCR 识别单张图像
    fn run_windows_ocr(&self, dynamic_img: image::DynamicImage) -> String {
        if self.ocr_engine.is_none() { return String::new(); }
        let engine = self.ocr_engine.as_ref().unwrap();

        let mut png_buffer = Cursor::new(Vec::new());
        if dynamic_img.write_to(&mut png_buffer, image::ImageFormat::Png).is_err() { return String::new(); }
        let png_bytes = png_buffer.into_inner();

        let stream = InMemoryRandomAccessStream::new().unwrap();
        let writer = DataWriter::CreateDataWriter(&stream).unwrap();
        if writer.WriteBytes(&png_bytes).is_err() { return String::new(); }
        if writer.StoreAsync().unwrap().get().is_err() { return String::new(); }
        if writer.FlushAsync().unwrap().get().is_err() { return String::new(); }
        if writer.DetachStream().is_err() { return String::new(); }
        if stream.Seek(0).is_err() { return String::new(); }

        let decoder = match BitmapDecoder::CreateAsync(&stream) {
             Ok(op) => match op.get() { Ok(d) => d, Err(_) => return String::new() },
             Err(_) => return String::new(),
        };
        let software_bitmap = match decoder.GetSoftwareBitmapAsync() {
             Ok(op) => match op.get() { Ok(b) => b, Err(_) => return String::new() },
             Err(_) => return String::new(),
        };
        let result = match engine.RecognizeAsync(&software_bitmap) {
             Ok(op) => match op.get() { Ok(res) => res, Err(_) => return String::new() },
             Err(_) => return String::new(),
        };
        
        let mut full_text = String::new();
        if let Ok(lines) = result.Lines() {
            for line in lines {
                if let Ok(text) = line.Text() { full_text.push_str(&text.to_string()); }
            }
        }
        full_text.replace(|c: char| c.is_whitespace(), "")
    }

    pub fn get_text_from_area(&self, rect: [i32; 4]) -> String {
         let x = rect[0]; 
         let y = rect[1];
         let w = (rect[2] - rect[0]).max(1);
         let h = (rect[3] - rect[1]).max(1);
         
         let screens = Screen::all().unwrap_or_default();
         let screen = match screens.first() { Some(s) => s, None => return String::new() };
         
         let captured_data = match screen.capture_area(x, y, w as u32, h as u32) {
             Ok(img) => img,
             Err(_) => return String::new(),
         };

         // 1. 基础转换
         let rgba_img = image::RgbaImage::from_raw(captured_data.width(), captured_data.height(), captured_data.into_raw()).unwrap();
         let dynamic_img = image::DynamicImage::ImageRgba8(rgba_img);

         // 2. 🔥 2倍放大：Lanczos3 采样能有效平滑艺术字边缘
         let scaled_img = dynamic_img.resize(w as u32 * 2, h as u32 * 2, image::imageops::FilterType::Lanczos3);
         
         // 3. 🔥 多重曝光 OCR 策略
         let mut results = Vec::new();

         // 策略 A: 强二值化 (阈值 200)
         let mut luma_high = scaled_img.grayscale().into_luma8();
         for pixel in luma_high.pixels_mut() { pixel[0] = if pixel[0] > 200 { 255 } else { 0 }; }
         results.push(self.run_windows_ocr(image::DynamicImage::ImageLuma8(luma_high)));

         // 策略 B: 中等二值化 (阈值 140)
         let mut luma_mid = scaled_img.grayscale().into_luma8();
         for pixel in luma_mid.pixels_mut() { pixel[0] = if pixel[0] > 140 { 255 } else { 0 }; }
         results.push(self.run_windows_ocr(image::DynamicImage::ImageLuma8(luma_mid)));

         // 策略 C: 原色缩放图
         results.push(self.run_windows_ocr(scaled_img.clone()));

         // 4. 合并所有识别到的文本块
         let final_text = results.join(" ");
         final_text
    }

    fn check_text_anchor(&self, rect: [i32; 4], expected: &str) -> bool {
        let output = self.get_text_from_area(rect);
        output.contains(expected)
    }

    pub fn debug_ocr_file(&self, file_path: &str, expected_contain: &str) {
        println!("📂 [本地测试] 加载: {}", file_path);
        if !Path::new(file_path).exists() { return; }
        let dynamic_img = image::open(file_path).expect("加载失败");
        let output = self.run_windows_ocr(dynamic_img);
        println!("📝 结果: [{}] | 期望: [{}] -> {}", output, expected_contain, output.contains(expected_contain));
    }

    fn check_color_anchor(&self, pos: [i32; 2], expected_hex: &str, tolerance: u8) -> bool {
        let x = pos[0]; let y = pos[1];
        let screens = Screen::all().unwrap_or_default();
        let screen = match screens.first() { Some(s) => s, None => return false };
        let image = match screen.capture_area(x, y, 1, 1) { Ok(img) => img, Err(_) => return false };
        let data = image.as_raw();
        if data.len() < 3 { return false; }
        let (r, g, b) = (data[0], data[1], data[2]);
        let expected_rgb = hex::decode(expected_hex.trim_start_matches('#')).unwrap_or(vec![0,0,0]);
        let diff = (r as i16 - expected_rgb[0] as i16).abs() + (g as i16 - expected_rgb[1] as i16).abs() + (b as i16 - expected_rgb[2] as i16).abs();
        diff <= (tolerance as i16 * 3)
    }

    fn perform_click(&self, x: i32, y: i32) {
        if let Ok(mut bot) = self.driver.lock() {
            bot.move_to_humanly(x as u16, y as u16, 0.6);
            bot.click_humanly(true, false, 0); 
        }
    }
}

// ==========================================
// 3. 导航引擎
// ==========================================
pub struct NavEngine {
    scenes: HashMap<String, Scene>,
    interface: GameInterface,
}

impl NavEngine {
    pub fn new(file_path: &str, driver: Arc<Mutex<HumanDriver>>) -> Self {
        let content = fs::read_to_string(file_path).expect("无法读取 TOML");
        let root: TomlRoot = toml::from_str(&content).expect("TOML 解析错误");
        let mut map = HashMap::new();
        for s in root.scenes { map.insert(s.id.clone(), s); }
        Self { scenes: map, interface: GameInterface::new(driver) }
    }

    pub fn test_ocr_on_file(&self, filename: &str, expected: &str) {
        self.interface.debug_ocr_file(filename, expected);
    }

    pub fn ocr_area(&self, rect: [i32; 4]) -> String {
        self.interface.get_text_from_area(rect)
    }

    fn get_match_score(&self, target_id: &str) -> usize {
        if let Some(scene) = self.scenes.get(target_id) {
            if scene.anchors.is_none() { return 0; }
            let anchors = scene.anchors.as_ref().unwrap();
            let mut score = 0;
            let mut total_checks = 0;
            if let Some(texts) = &anchors.text {
                for t in texts {
                    total_checks += 1;
                    if self.interface.check_text_anchor(t.rect, &t.val) { score += 1; }
                }
            }
            if let Some(colors) = &anchors.color {
                for c in colors {
                    total_checks += 1;
                    if self.interface.check_color_anchor(c.pos, &c.val, c.tol) { score += 1; }
                }
            }
            let passed = match scene.logic.to_lowercase().as_str() {
                "or" => score > 0,              
                _ => score == total_checks && total_checks > 0, 
            };
            if passed { return score; }
        }
        0
    }

    pub fn identify_current_scene(&self, hint: Option<&str>) -> Option<String> {
        println!("👀 扫描当前界面...");
        if let Some(target_id) = hint {
            if self.get_match_score(target_id) > 0 {
                println!("✅ 命中预期目标: [{}]", target_id);
                return Some(target_id.to_string());
            }
        }
        let mut best_match: Option<String> = None;
        let mut max_score = 0;
        for (id, _) in &self.scenes {
            if let Some(h) = hint { if h == id { continue; } }
            let score = self.get_match_score(id);
            if score > 0 && score > max_score {
                max_score = score;
                best_match = Some(id.clone());
            }
        }
        if let Some(id) = &best_match { println!("✅ 定位: [{}] (得分: {})", id, max_score); }
        best_match
    }

    fn wait_for_scene(&self, target_id: &str, timeout_ms: u64) -> bool {
        let start = Instant::now();
        println!("    👀 确认进入 [{}]...", target_id);
        while start.elapsed().as_millis() < timeout_ms as u128 {
            if self.get_match_score(target_id) > 0 {
                println!("    ✅ 确认到达 (耗时 {}ms)", start.elapsed().as_millis());
                return true;
            }
            thread::sleep(Duration::from_millis(200));
        }
        println!("    ⚠️ 等待超时 [{}]", target_id);
        false
    }

    pub fn navigate(&self, target_id: &str) -> NavResult {
        let start_id = match self.identify_current_scene(None) {
            Some(id) => id,
            None => { println!("❌ 无法定位起点"); return NavResult::Failed; }
        };
        if start_id == target_id {
            println!("✅ 已在目标位置");
            return NavResult::Success;
        }
        println!("🤖 规划路径: [{}] -> [{}]", start_id, target_id);
        let path = match self.find_path(&start_id, target_id) {
            Some(p) => p,
            None => { println!("❌ 无路可走"); return NavResult::Failed; }
        };
        for (i, step) in path.iter().enumerate() {
            println!("\n➡️  [步骤 {}/{}] 点击 -> [{}]", i+1, path.len(), step.target);
            self.interface.perform_click(step.coords[0], step.coords[1]);
            
            // ✨ 核心修改：检查是否需要移交控制权
            // 如果 TOML 里写了 handler = "xxx"，或者它是无锚点的虚拟节点，则移交
            let (should_handover, handler_name) = if let Some(s) = self.scenes.get(&step.target) {
                // 如果有 handler 字段，或者没有锚点，都视为需要移交
                (s.handler.is_some() || s.anchors.is_none(), s.handler.clone())
            } else { 
                (false, None) 
            };

            if should_handover {
                println!("🚀 到达托管节点 [{}]，触发处理器: {:?}", step.target, handler_name);
                thread::sleep(Duration::from_millis(step.post_delay));
                // 将 handler 名称一并返回给 main
                return NavResult::Handover(step.target.clone(), handler_name);
            }

            let timeout = if step.post_delay < 2000 { 2000 } else { step.post_delay };
            if !self.wait_for_scene(&step.target, timeout) {
                println!("❌ 导航中断: 未能进入 [{}]", step.target);
                return NavResult::Failed;
            }
            thread::sleep(Duration::from_millis(300));
        }
        println!("✅ 导航完成");
        NavResult::Success
    }

    fn find_path(&self, start: &str, target: &str) -> Option<Vec<Transition>> {
        if start == target { return Some(vec![]); }
        let mut queue = VecDeque::from([start.to_string()]);
        let mut came_from: HashMap<String, (String, Transition)> = HashMap::new();
        let mut visited = vec![start.to_string()];
        while let Some(curr) = queue.pop_front() {
            if curr == target {
                let mut path = vec![];
                let mut p = target.to_string();
                while p != start {
                    if let Some((prev, trans)) = came_from.get(&p) { path.push(trans.clone()); p = prev.clone(); }
                }
                path.reverse(); return Some(path);
            }
            if let Some(scene) = self.scenes.get(&curr) {
                if let Some(trans) = &scene.transitions {
                    for t in trans {
                        if !visited.contains(&t.target) {
                            visited.push(t.target.clone()); queue.push_back(t.target.clone()); came_from.insert(t.target.clone(), (curr.clone(), t.clone()));
                        }
                    }
                }
            }
        }
        None
    }
}
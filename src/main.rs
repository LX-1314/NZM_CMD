// src/main.rs
use nzm_cmd::hardware::InputDevice;
use nzm_cmd::human::HumanDriver;
use nzm_cmd::nav::{NavEngine, NavResult};
use nzm_cmd::tower_defense::TowerDefenseApp;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use clap::Parser;
use screenshots::Screen; 

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// 硬件串口名称 (例如: COM9, /dev/ttyUSB0)
    #[arg(short, long, default_value = "COM3")]
    port: String,

    /// 导航目标界面名称 (例如: "空间站普通", "空间站炼狱")
    #[arg(short, long, default_value = "空间站普通")]
    target: String,

    /// 运行测试模式 (可选: input, screen, ocr)
    #[arg(long)]
    test: Option<String>,
}

fn main() {
    let args = Args::parse();

    println!("========================================");
    println!("🚀 NZM_CMD 智能控制中心");
    println!("📍 端口: {}", args.port);
    if let Some(t) = &args.test {
        println!("🔧 模式: 测试 ({})", t);
    } else {
        println!("🎯 目标: {}", args.target);
    }
    println!("========================================");

    // 1. 硬件驱动初始化
    let (sw, sh) = (1920, 1080);
    let driver_arc = match InputDevice::new(&args.port, 115200, sw, sh) {
        Ok(d) => Arc::new(Mutex::new(d)),
        Err(e) => {
            println!("⚠️ 警告: 无法连接硬件 ({})", e);
            println!("⚠️ 进入无硬件模拟模式");
            unsafe { std::mem::transmute(Arc::new(Mutex::new(()))) } 
        }
    };

    // 启动心跳
    let hb = Arc::clone(&driver_arc);
    thread::spawn(move || loop {
        if let Ok(mut d) = hb.lock() { d.heartbeat(); }
        thread::sleep(Duration::from_secs(1));
    });

    // 2. 初始化驱动与引擎
    let human_driver = Arc::new(Mutex::new(
        HumanDriver::new(Arc::clone(&driver_arc), sw/2, sh/2)
    ));

    let engine = Arc::new(NavEngine::new("ui_map.toml", Arc::clone(&human_driver)));

    // ==========================================
    // 🔍 测试模式 (测试完直接退出)
    // ==========================================
    if let Some(mode) = args.test.as_deref() {
        println!("⏳ 5秒后开始执行 [{}] 测试...", mode);
        thread::sleep(Duration::from_secs(5));
        match mode {
            "input" => run_input_test(human_driver),
            "screen" => run_screen_test(),
            "ocr" => run_ocr_test(engine),
            _ => println!("❌ 未知测试模式"),
        }
        return; 
    }

    // ==========================================
    // 🚀 自动化循环 (正常业务流程)
    // ==========================================
    println!("✅ 引擎就绪，5秒后开始自动化循环...");
    thread::sleep(Duration::from_secs(5));

    // ✨ 核心修改：无限循环
    loop {
        println!("\n🔄 [主控] 正在导航至: {}...", args.target);
        
        // 执行导航
        let nav_result = engine.navigate(&args.target);

        match nav_result {
            NavResult::Handover(scene_id) => {
                println!("⚔️ [主控] 导航成功: [{}] -> 启动塔防逻辑", scene_id);
                
                // 1. 初始化塔防 APP
                let mut td_app = TowerDefenseApp::new(Arc::clone(&human_driver), Arc::clone(&engine));
                
                // 2. 动态生成文件名
                let map_file = format!("{}地图.json", scene_id);
                let strategy_file = format!("{}策略.json", scene_id);
                let traps_file = "traps_config.json";

                println!("📂 加载配置: {} | {}", map_file, strategy_file);

                // 3. 运行塔防逻辑 (阻塞直到游戏结束)
                td_app.run(&map_file, &strategy_file, traps_file);

                // 4. 运行结束，准备下一轮
                println!("🎉 本局结束，5秒后重新开始循环...");
                thread::sleep(Duration::from_secs(5));
            }
            
            NavResult::Failed => {
                println!("❌ [主控] 导航失败，执行重置操作 (ESC)...");
                
                // 尝试按下 ESC (HID code 0x29) 关闭可能的弹窗或菜单
                if let Ok(mut human) = human_driver.lock() {
                    if let Ok(mut dev) = human.device.lock() {
                        // 0x29 是键盘 ESC 的 HID 码
                        dev.key_down(0x29, 0);
                    }
                    thread::sleep(Duration::from_millis(100));

                    if let Ok(mut dev) = human.device.lock() {
                        dev.key_up(); // 松开所有按键
                    }
                }
                
                println!("⏳ 等待界面重置 (3秒)...");
                thread::sleep(Duration::from_secs(3));
                // 循环会自动 continue，重试导航
            }
            
            NavResult::Success => {
                println!("✅ [主控] 导航到达终点 (无后续逻辑)，等待重置...");
                thread::sleep(Duration::from_secs(5));
                // 如果是单纯的领取任务，这里可以 continue 继续下一轮
            }
        }
    }
}

// ----------------------------------------------------------------
// 🛠️ 测试函数实现
// ----------------------------------------------------------------

fn run_input_test(driver: Arc<Mutex<HumanDriver>>) {
    println!("Testing Mouse & Keyboard...");
    if let Ok(mut d) = driver.lock() {
        // 1. 鼠标方形移动测试
        println!("-> 移动鼠标 (矩形轨迹)");
        let start_x = 500;
        let start_y = 500;
        d.move_to_humanly(start_x, start_y, 0.5);
        d.move_to_humanly(start_x + 300, start_y, 0.5);
        d.move_to_humanly(start_x + 300, start_y + 300, 0.5);
        d.move_to_humanly(start_x, start_y + 300, 0.5);
        d.move_to_humanly(start_x, start_y, 0.5);

        // 2. 点击测试
        println!("-> 执行点击 (Click)");
        d.click_humanly(true, false, 0);
        thread::sleep(Duration::from_millis(500));

        // 3. 键盘输入测试
        println!("-> 模拟键盘输入 'hello 123'");
        d.type_humanly("hello 123", 60.0);
    }
    println!("Done.");
}

fn run_screen_test() {
    println!("Testing Screen Capture...");
    let start = Instant::now();
    let screens = Screen::all().unwrap_or_default();
    
    if let Some(screen) = screens.first() {
        println!("-> 检测到屏幕: {}x{}", screen.display_info.width, screen.display_info.height);
        match screen.capture() {
            Ok(image) => {
                let path = "debug_screenshot.png";
                image.save(path).unwrap();
                println!("✅ 截图成功! 已保存至: {} (耗时 {}ms)", path, start.elapsed().as_millis());
                println!("   请打开图片确认颜色和内容是否正常。");
            },
            Err(e) => println!("❌ 截图失败: {}", e),
        }
    } else {
        println!("❌ 未检测到显示器");
    }
}

fn run_ocr_test(engine: Arc<NavEngine>) {
    println!("Testing OCR Function...");
    // 定义一个测试区域 (例如屏幕左上角的一块区域，通常包含HUD信息)
    // 这里取 x=100, y=100, w=400, h=100
    let rect = [100, 100, 500, 200]; 
    
    println!("-> 正在识别区域: {:?}", rect);
    let start = Instant::now();
    let text = engine.ocr_area(rect);
    
    println!("----------------------------------------");
    println!("⏱️ 耗时: {} ms", start.elapsed().as_millis());
    println!("📝 识别结果: [{}]", text);
    println!("----------------------------------------");
    
    if text.is_empty() {
        println!("⚠️ 警告: 识别结果为空，请确认该区域有文字。");
    }
}
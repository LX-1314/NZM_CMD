// src/main.rs
use clap::Parser;
use nzm_cmd::daily_routine::DailyRoutineApp; // 引入日活模块
use nzm_cmd::hardware::{create_driver, DriverType, InputDriver};
use nzm_cmd::human::HumanDriver;
use nzm_cmd::nav::{NavEngine, NavResult};
use nzm_cmd::tower_defense::TowerDefenseApp;
use screenshots::Screen;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short, long, default_value = "COM3")]
    port: String,

    #[arg(short, long, default_value = "空间站普通")]
    target: String,

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

    let (sw, sh) = (1920, 1080);

    let driver_type = if args.port.to_uppercase() == "SOFT" {
        DriverType::Software
    } else {
        DriverType::Hardware
    };

    let driver_box: Box<dyn InputDriver> = match create_driver(driver_type, &args.port, sw, sh) {
        Ok(d) => d,
        Err(e) => {
            println!("⚠️ 警告: 无法初始化驱动 ({})", e);
            println!("⚠️ 尝试回退到 [软件模拟模式]...");
            create_driver(DriverType::Software, "", sw, sh).unwrap()
        }
    };

    let driver_arc: Arc<Mutex<Box<dyn InputDriver>>> = Arc::new(Mutex::new(driver_box));

    let hb = Arc::clone(&driver_arc);
    thread::spawn(move || loop {
        if let Ok(mut d) = hb.lock() {
            d.heartbeat();
        }
        thread::sleep(Duration::from_secs(1));
    });

    let human_driver = Arc::new(Mutex::new(HumanDriver::new(
        Arc::clone(&driver_arc),
        sw / 2,
        sh / 2,
    )));

    let engine = Arc::new(NavEngine::new("ui_map.toml", Arc::clone(&human_driver)));

    if let Some(mode) = args.test.as_deref() {
        println!("⏳ 5秒后开始执行 [{}] 测试...", mode);
        thread::sleep(Duration::from_secs(5));
        match mode {
            "input" => run_input_test(human_driver),
            "screen" => run_screen_test(),
            "ocr" => run_ocr_test(engine),
            "scroll" => run_scroll_test(human_driver), // ✨ 新增这一行
            _ => println!("❌ 未知测试模式"),
        }
        return;
    }

    println!("✅ 引擎就绪，5秒后开始自动化循环...");
    thread::sleep(Duration::from_secs(5));

    loop {
        println!("\n🔄 [主控] 正在导航至: {}...", args.target);

        let nav_result = engine.navigate(&args.target);

        match nav_result {
            // ✨ 核心修改：接收 handler 参数
            NavResult::Handover(scene_id, handler_opt) => {
                println!("⚔️ [主控] 导航成功: [{}]", scene_id);

                // 如果 TOML 里没配置 handler，默认 fallback 到 "td" (塔防)
                // 这样兼容旧的配置文件
                let handler_key = handler_opt.as_deref().unwrap_or("td");

                match handler_key {
                    "daily" => {
                        println!("📅 [路由] 检测到 'daily' 标记，启动日活模块...");
                        let app =
                            DailyRoutineApp::new(Arc::clone(&human_driver), Arc::clone(&engine));
                        app.run();
                    }
                    "td" | _ => {
                        // 默认处理逻辑 (塔防)
                        println!("🏰 [路由] 启动塔防模块 (Handler: {})...", handler_key);
                        let mut td_app =
                            TowerDefenseApp::new(Arc::clone(&human_driver), Arc::clone(&engine));

                        let map_file = format!("{}地图.json", scene_id);
                        let strategy_file = format!("{}策略.json", scene_id);
                        let traps_file = "traps_config.json";

                        println!("📂 加载配置: {} | {}", map_file, strategy_file);
                        td_app.run(&map_file, &strategy_file, traps_file);
                    }
                }

                println!("🎉 本局任务结束，5秒后重新开始循环...");
                thread::sleep(Duration::from_secs(5));
            }

            NavResult::Failed => {
                println!("❌ [主控] 导航失败，执行重置操作 (ESC)...");

                if let Ok(mut human) = human_driver.lock() {
                    // 使用 unicode 转义避免字符字面量错误
                    human.key_hold('\u{1B}', 100);

                    if let Ok(mut dev) = human.device.lock() {
                        dev.key_down(0x29, 0);
                    }
                    thread::sleep(Duration::from_millis(100));
                    if let Ok(mut dev) = human.device.lock() {
                        dev.key_up();
                    }
                }

                println!("⏳ 等待界面重置 (3秒)...");
                thread::sleep(Duration::from_secs(3));
            }

            NavResult::Success => {
                println!("✅ [主控] 导航到达终点，等待重置...");
                thread::sleep(Duration::from_secs(5));
            }
        }
    }
}

// ... (测试函数 run_input_test, run_screen_test, run_ocr_test 保持不变) ...
fn run_input_test(driver: Arc<Mutex<HumanDriver>>) {
    println!("Testing Mouse & Keyboard...");
    if let Ok(mut d) = driver.lock() {
        println!("-> 移动鼠标 (矩形轨迹)");
        let start_x = 500;
        let start_y = 500;
        d.move_to_humanly(start_x, start_y, 0.5);
        d.move_to_humanly(start_x + 300, start_y, 0.5);
        d.move_to_humanly(start_x + 300, start_y + 300, 0.5);
        d.move_to_humanly(start_x, start_y + 300, 0.5);
        d.move_to_humanly(start_x, start_y, 0.5);

        println!("-> 执行点击 (Click)");
        d.click_humanly(true, false, 0);
        thread::sleep(Duration::from_millis(500));

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
        println!(
            "-> 检测到屏幕: {}x{}",
            screen.display_info.width, screen.display_info.height
        );
        match screen.capture() {
            Ok(image) => {
                let path = "debug_screenshot.png";
                image.save(path).unwrap();
                println!(
                    "✅ 截图成功! 已保存至: {} (耗时 {}ms)",
                    path,
                    start.elapsed().as_millis()
                );
            }
            Err(e) => println!("❌ 截图失败: {}", e),
        }
    } else {
        println!("❌ 未检测到显示器");
    }
}

fn run_ocr_test(engine: Arc<NavEngine>) {
    println!("Testing OCR Function...");
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


fn run_scroll_test(driver: Arc<Mutex<HumanDriver>>) {
    println!("Testing Mouse Scroll...");
    if let Ok(mut d) = driver.lock() {
        println!("-> 向下滚动 5 格 (Scroll Down)");
        // 负数通常是向下滚动
        // 每次 -120 是一格 (标准 Windows 定义)，或者根据驱动实现可能是 -1
        // 这里尝试传 -1 (因为 HardwareDriver 内部实现了累积，而 SoftwareDriver 调用 Enigo)
        // 建议先试小数值，比如 -5 代表滚动5次
        d.mouse_scroll(-5); 
        
        thread::sleep(Duration::from_secs(2));

        println!("-> 向上滚动 5 格 (Scroll Up)");
        d.mouse_scroll(5);
    }
    println!("Done.");
}
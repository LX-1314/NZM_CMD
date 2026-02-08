// src/daily_routine.rs
use crate::human::HumanDriver;
use crate::nav::NavEngine;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// 定义单个任务槽位的配置
struct TaskSlot {
    index: usize,
    /// 状态文字识别区域 [x1, y1, x2, y2]
    status_rect: [i32; 4],
    /// 刷新按钮坐标 (x, y)
    refresh_pos: (u16, u16),
}

pub struct DailyRoutineApp {
    driver: Arc<Mutex<HumanDriver>>,
    nav: Arc<NavEngine>,
    slots: Vec<TaskSlot>,
}

impl DailyRoutineApp {
    pub fn new(driver: Arc<Mutex<HumanDriver>>, nav: Arc<NavEngine>) -> Self {
        // 根据您提供的坐标配置 4 个任务槽
        let slots = vec![
            TaskSlot {
                index: 1,
                status_rect: [559, 914, 768, 963],
                refresh_pos: (784, 311),
            },
            TaskSlot {
                index: 2,
                status_rect: [899, 901, 1104, 977],
                refresh_pos: (1124, 314),
            },
            TaskSlot {
                index: 3,
                status_rect: [1238, 901, 1439, 968],
                refresh_pos: (1465, 318),
            },
            TaskSlot {
                index: 4,
                status_rect: [1560, 895, 1792, 968],
                refresh_pos: (1804, 316),
            },
        ];

        Self { driver, nav, slots }
    }

    /// 执行日活逻辑主入口
    pub fn run(&self) {
        println!("📅 [Daily] 开始执行日活任务逻辑...");
        
        // 最大轮次，防止无限刷新把钱刷光了
        let max_rounds = 10; 

        for round in 1..=max_rounds {
            println!("\n🔄 [Daily] 第 {}/{} 轮扫描...", round, max_rounds);
            
            let mut need_retry = false;
            
            // 遍历 4 个任务槽
            for slot in &self.slots {
                let processed = self.process_slot(slot);
                if processed {
                    need_retry = true;
                }
                // 槽位间稍微停顿，看起来更像人
                thread::sleep(Duration::from_millis(500)); 
            }

            if !need_retry {
                println!("✅ [Daily] 所有任务已完成或已领取！");
                break;
            }

            // 如果本轮有操作（领取或刷新），等待界面动画刷新后继续
            println!("⏳ 等待任务列表刷新 (2秒)...");
            thread::sleep(Duration::from_secs(2));
        }

        println!("🏁 [Daily] 日活流程结束。");
    }

    /// 处理单个槽位，返回 true 表示进行了操作（需要进入下一轮检查）
// src/daily_routine.rs

    fn process_slot(&self, slot: &TaskSlot) -> bool {
        // 1. OCR 识别状态
        let text = self.nav.ocr_area(slot.status_rect);
        // 去除空格和换行，防止 OCR 识别出 "已 完 成" 导致匹配失败
        let clean_text = text.replace(|c: char| c.is_whitespace(), ""); 

        println!("   📝 槽位[{}] 识别结果: [{}]", slot.index, clean_text);

        // =========================================================
        // 逻辑判断 (注意顺序：先排除终态，再判断操作)
        // =========================================================

        // 1. 【终态】已完成 / 已领取
        // ⚠️ 必须放在最前面！因为 "已领取" 包含 "领取" 字样
        if clean_text.contains("已完成") || clean_text.contains("已领取") {
            println!("      -> ✅ 任务已结束，跳过。");
            return false; // 不做操作
        }

        // 2. 【可领取】
        if clean_text.contains("领取") {
            println!("      -> 🎉 发现可领取奖励，执行领取流程...");
            if let Ok(mut d) = self.driver.lock() {
                // A. 点击状态文字中心 (即领取按钮)
                let cx = (slot.status_rect[0] + slot.status_rect[2]) / 2;
                let cy = (slot.status_rect[1] + slot.status_rect[3]) / 2;
                d.move_to_humanly(cx as u16, cy as u16, 0.5);
                d.click_humanly(true, false, 0);

                // B. 处理奖励弹窗 (按空格跳过)
                println!("      -> ⏳ 等待弹窗并按空格跳过...");
                thread::sleep(Duration::from_millis(1000)); // 等待动画
                d.key_click(' '); 
                thread::sleep(Duration::from_millis(1000));
                d.key_click(' '); // 连按两次防止漏掉
            }
            return true; // 做了操作，需要重试扫描
        }

        // 3. 【未完成】需要刷新
        if clean_text.contains("去完成") || clean_text.contains("未完成") {
            println!("      -> ⚠️ 任务未完成，点击刷新 ({}, {})...", slot.refresh_pos.0, slot.refresh_pos.1);
            if let Ok(mut d) = self.driver.lock() {
                // 点击对应的刷新按钮
                d.move_to_humanly(slot.refresh_pos.0, slot.refresh_pos.1, 0.5);
                d.click_humanly(true, false, 0);
                
                // 刷新后的短暂冷却
                thread::sleep(Duration::from_millis(500));
            }
            return true; // 做了操作，需要重试扫描
        }
        
        // 4. 【兜底】识别为空或其他未知状态
        if clean_text.is_empty() {
             println!("      -> ⚪ 识别为空 (可能是图标/过暗)，暂跳过");
             return false;
        }

        println!("      -> ❓ 未知状态，跳过");
        false
    }
}
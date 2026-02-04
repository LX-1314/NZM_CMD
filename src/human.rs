// src/human.rs
use crate::hardware::InputDevice; // 👈 路径变更
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use rand::Rng;
use rand_distr::{Normal, Distribution};

pub struct HumanDriver {
    pub device: Arc<Mutex<InputDevice>>,
    pub cur_x: f32,
    pub cur_y: f32,
}

impl HumanDriver {
    /// 初始化拟人化驱动器
    pub fn new(device: Arc<Mutex<InputDevice>>, start_x: u16, start_y: u16) -> Self {
        Self {
            device,
            cur_x: start_x as f32,
            cur_y: start_y as f32,
        }
    }

    // ==========================================
    // 1. 基础输入原子操作 (原子层)
    // ==========================================

    /// 内部辅助：字符转 HID 键码
    fn char_to_keycode(&self, ch: char) -> u8 {
        match ch.to_ascii_lowercase() {
            'a'..='z' => ch.to_ascii_lowercase() as u8 - b'a' + 0x04,
            '1'..='9' => ch as u8 - b'1' + 0x1E,
            '0' => 0x27,
            ' ' => 0x2C,
            _ => 0,
        }
    }

    /// 🔥 【键盘长按】
    /// 允许指定按下的毫秒数。如果是 0，则执行一次极短的点击。
    pub fn key_hold(&mut self, ch: char, ms: u64) {
        let keycode = self.char_to_keycode(ch);
        if keycode != 0 {
            if let Ok(mut dev) = self.device.lock() {
                dev.key_down(keycode, 0);
            }
            
            // 如果 ms 为 0，模拟一个非常短的物理接触
            let hold_time = if ms > 0 { ms } else { rand::thread_rng().gen_range(20..45) };
            thread::sleep(Duration::from_millis(hold_time));

            if let Ok(mut dev) = self.device.lock() {
                dev.key_up();
            }
        }
    }

    /// 【拟人化按键点击】 (短按)
    pub fn key_click(&mut self, ch: char) {
        // 模拟真实按键点击通常在 30-70ms 之间
        let jitter = rand::thread_rng().gen_range(35..70);
        self.key_hold(ch, jitter);
    }

    /// 🔥 【模拟鼠标滚轮】
    /// delta: 120 的倍数，正数为向上滚，负数为向下滚
    pub fn mouse_scroll(&mut self, delta: i32) {
        if let Ok(mut dev) = self.device.lock() {
            // 在 lib.rs 中 mouse_move 的第三个参数通常对应滚轮字节
            dev.mouse_move(0, 0, delta as i8);
        }
        // 滚轮后稍微停顿符合人体工程学
        thread::sleep(Duration::from_millis(100));
    }

    /// 🔥 【相对移动】
    /// 用于在当前位置基础上进行微调或防掉线微动
    pub fn move_relative(&mut self, dx: i32, dy: i32) {
        if let Ok(mut dev) = self.device.lock() {
            dev.mouse_move(dx, dy, 0);
        }
        self.cur_x += dx as f32;
        self.cur_y += dy as f32;
    }

    // ==========================================
    // 2. 高级拟人化行为 (行为层)
    // ==========================================

    /// 【高级拟人移动】
    pub fn move_to_humanly(&mut self, target_x: u16, target_y: u16, duration_sec: f32) {
        let mut rng = rand::thread_rng();
        let start = (self.cur_x, self.cur_y);
        
        let end = (
            target_x as f32 + rng.gen_range(-2.0..2.0),
            target_y as f32 + rng.gen_range(-2.0..2.0)
        );

        let ctrl1 = (
            start.0 + (end.0 - start.0) * 0.2 + rng.gen_range(-40.0..40.0),
            start.1 + (end.1 - start.1) * 0.2 + rng.gen_range(-40.0..40.0)
        );
        let ctrl2 = (
            start.0 + (end.0 - start.0) * 0.8 + rng.gen_range(-20.0..60.0),
            start.1 + (end.1 - start.1) * 0.8 + rng.gen_range(-20.0..60.0)
        );

        let steps = (duration_sec * 80.0) as u32; 
        let interval = Duration::from_secs_f32(duration_sec / steps as f32);

        for i in 0..=steps {
            let t_linear = i as f32 / steps as f32;
            let t_eased = Self::ease_in_out_cubic(t_linear);
            let (px, py) = Self::bezier_cubic(t_eased, start, ctrl1, ctrl2, end);
            
            if let Ok(mut dev) = self.device.lock() {
                dev.mouse_abs(px as u16, py as u16);
            }
            thread::sleep(interval);
        }

        self.cur_x = end.0;
        self.cur_y = end.1;
    }

    /// 【拟人化鼠标点击】
    /// 增加 hold_ms 参数以支持长按点击（如蓄力）
    pub fn click_humanly(&mut self, left: bool, right: bool, hold_ms: u64) {
        let mut rng = rand::thread_rng();
        if let Ok(mut dev) = self.device.lock() {
            dev.mouse_down(left, right);
            
            let sleep_time = if hold_ms > 0 { hold_ms } else { rng.gen_range(30..75) };
            thread::sleep(Duration::from_millis(sleep_time));
            
            dev.mouse_up();
        }
    }

    pub fn double_click_humanly(&mut self, left: bool, right: bool) {
        let mut rng = rand::thread_rng();
        
        // 第一次点击
        self.click_humanly(left, right, 0);
        
        // 两次点击之间的关键间隔：拟人化的短促停顿 (通常 40-90ms 是双击的黄金时间)
        let interval = rng.gen_range(40..85);
        thread::sleep(Duration::from_millis(interval));
        
        // 第二次点击
        self.click_humanly(left, right, 0);
    }

    /// 【拟人化打字】
    pub fn type_humanly(&mut self, text: &str, base_wpm: f32) {
        let base_delay_ms = 60.0 / (base_wpm * 5.0) * 1000.0;
        let normal_dist = Normal::new(base_delay_ms, base_delay_ms * 0.3).unwrap();
        let mut rng = rand::thread_rng();

        for ch in text.chars() {
            // 直接复用我们新写的 key_click
            self.key_click(ch);

            // 字符间的随机停顿
            let delay = normal_dist.sample(&mut rng).max(10.0) as u64;
            thread::sleep(Duration::from_millis(delay));
        }
    }

    // ==========================================
    // 3. 数学辅助函数 (数学层)
    // ==========================================

    fn ease_in_out_cubic(t: f32) -> f32 {
        if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0 }
    }

    fn bezier_cubic(t: f32, p0: (f32, f32), p1: (f32, f32), p2: (f32, f32), p3: (f32, f32)) -> (f32, f32) {
        let u = 1.0 - t;
        let tt = t * t;
        let uu = u * u;
        let x = uu * u * p0.0 + 3.0 * uu * t * p1.0 + 3.0 * u * tt * p2.0 + tt * t * p3.0;
        let y = uu * u * p0.1 + 3.0 * uu * t * p1.1 + 3.0 * u * tt * p2.1 + tt * t * p3.1;
        (x, y)
    }
}
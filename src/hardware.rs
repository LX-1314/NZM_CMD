use byteorder::{LittleEndian, WriteBytesExt};
// ✨ 引入 enigo 0.6.1 的正确 Traits 和 Structs
use enigo::{
    Direction, Enigo, Key, Keyboard, Mouse, Settings, Coordinate,
    Button, // 0.6.1 使用 Button 而不是 MouseButton
};
use serialport::SerialPort;
use std::io::Write;
use std::thread;
use std::time::Duration;

// ==========================================
// 1. 公共接口定义 (Trait)
// ==========================================
pub trait InputDriver: Send + Sync {
    fn heartbeat(&mut self);
    fn mouse_abs(&mut self, x: u16, y: u16);
    fn mouse_move(&mut self, dx: i32, dy: i32, wheel: i8);
    fn mouse_down(&mut self, left: bool, right: bool);
    fn mouse_up(&mut self);
    fn key_down(&mut self, keycode: u8, modifier: u8);
    fn key_up(&mut self);
    fn switch_identity(&mut self, index: u8);
}

// ==========================================
// 2. 硬件驱动实现 (Serial Port)
// ==========================================
// ... (HardwareDriver 部分保持不变，为了节省篇幅略去，请保留原来的代码) ...
const FRAME_HEAD: u8 = 0xAA;
const FRAME_TAIL: u8 = 0x55;

#[repr(u8)]
enum EventType {
    Keyboard = 0x01,
    MouseRel = 0x02,
    MouseAbs = 0x03,
    System = 0x04,
}

#[repr(u8)]
enum SystemCmd {
    SetId = 0x10,
    Heartbeat = 0xFF,
}

pub struct HardwareDriver {
    port: Box<dyn SerialPort>,
    pub screen_w: u16,
    pub screen_h: u16,
}

impl HardwareDriver {
    pub fn new(port_name: &str, baud_rate: u32, screen_w: u16, screen_h: u16) -> Result<Self, String> {
        let port = serialport::new(port_name, baud_rate)
            .timeout(Duration::from_millis(100))
            .open()
            .map_err(|e| format!("无法打开串口 {}: {}", port_name, e))?;

        Ok(Self { port, screen_w, screen_h })
    }

    fn send_raw(&mut self, event_type: EventType, b: [u8; 6], delay_ms: u16) {
        let mut frame = Vec::with_capacity(11);
        frame.push(FRAME_HEAD);
        frame.push(event_type as u8);
        frame.extend_from_slice(&b);
        frame.write_u16::<LittleEndian>(delay_ms).unwrap();
        frame.push(FRAME_TAIL);

        let _ = self.port.write_all(&frame);
        let _ = self.port.flush();
        thread::sleep(Duration::from_millis(4));
    }
}

// 必须手动实现 Sync，因为 serialport 的 Box 对象默认不是 Sync 的
unsafe impl Sync for HardwareDriver {}

impl InputDriver for HardwareDriver {
    fn heartbeat(&mut self) {
        let mut b = [0u8; 6];
        b[0] = SystemCmd::Heartbeat as u8;
        self.send_raw(EventType::System, b, 0);
    }

    fn switch_identity(&mut self, index: u8) {
        let mut b = [0u8; 6];
        b[0] = SystemCmd::SetId as u8;
        b[1] = index;
        self.send_raw(EventType::System, b, 0);
    }

    fn mouse_abs(&mut self, x: u16, y: u16) {
        let tx = ((x as f32 / self.screen_w as f32) * 32767.0) as u16;
        let ty = ((y as f32 / self.screen_h as f32) * 32767.0) as u16;
        let tx = tx.clamp(10, 32757);
        let ty = ty.clamp(10, 32757);

        let mut b = [0u8; 6];
        b[2] = (tx & 0xFF) as u8;
        b[3] = ((tx >> 8) & 0xFF) as u8;
        b[4] = (ty & 0xFF) as u8;
        b[5] = ((ty >> 8) & 0xFF) as u8;
        self.send_raw(EventType::MouseAbs, b, 0);
    }

    fn mouse_move(&mut self, dx: i32, dy: i32, wheel: i8) {
        if wheel != 0 {
            self.send_raw(EventType::MouseRel, [0, wheel as u8, 0, 0, 0, 0], 0);
        }
        let max_step = 127;
        let mut cur_dx = dx;
        let mut cur_dy = dy;

        while cur_dx != 0 || cur_dy != 0 {
            let step_x = if cur_dx > 0 { cur_dx.min(max_step) } else { cur_dx.max(-max_step) };
            let step_y = if cur_dy > 0 { cur_dy.min(max_step) } else { cur_dy.max(-max_step) };
            
            let bx = (step_x as i16).to_le_bytes();
            let by = (step_y as i16).to_le_bytes();
            
            self.send_raw(EventType::MouseRel, [0, 0, bx[0], bx[1], by[0], by[1]], 0);
            
            cur_dx -= step_x;
            cur_dy -= step_y;
        }
    }

    fn mouse_down(&mut self, left: bool, right: bool) {
        let mut mask = 0;
        if left { mask |= 0x01; }
        if right { mask |= 0x02; }
        self.send_raw(EventType::MouseRel, [mask, 0, 0, 0, 0, 0], 0);
    }

    fn mouse_up(&mut self) {
        self.send_raw(EventType::MouseRel, [0, 0, 0, 0, 0, 0], 0);
    }

    fn key_down(&mut self, keycode: u8, modifier: u8) {
        self.send_raw(EventType::Keyboard, [keycode, 0x00, modifier, 0, 0, 0], 0);
    }

    fn key_up(&mut self) {
        self.send_raw(EventType::Keyboard, [0, 0x80, 0, 0, 0, 0], 0);
    }
}

// ==========================================
// 3. 软件驱动实现 (Software / Enigo 0.6.1)
// ==========================================
pub struct SoftwareDriver {
    enigo: Enigo,
    pub screen_w: u16,
    pub screen_h: u16,
    last_key: Option<Key>,
}

// 同样需要手动实现 Sync，因为 Enigo 内部实现可能没显式标记
unsafe impl Sync for SoftwareDriver {}

impl SoftwareDriver {
    pub fn new(screen_w: u16, screen_h: u16) -> Self {
        // Enigo 0.6.1 初始化需要 Settings
        // 使用 unwrap 是因为默认设置通常不会失败
        Self {
            enigo: Enigo::new(&Settings::default()).unwrap(),
            screen_w,
            screen_h,
            last_key: None,
        }
    }

    // 修复后的映射函数，适配 Enigo 0.6.1
    fn hid_to_enigo(&self, hid: u8) -> Option<Key> {
        match hid {
            0x04..=0x1D => { // a-z
                let c = (b'a' + (hid - 0x04)) as char;
                Some(Key::Unicode(c)) // 0.6.1 使用 Unicode
            },
            0x1E..=0x27 => { // 1-9, 0
                let c = if hid == 0x27 { '0' } else { (b'1' + (hid - 0x1E)) as char };
                Some(Key::Unicode(c))
            },
            0x28 => Some(Key::Return),
            0x29 => Some(Key::Escape),
            0x2A => Some(Key::Backspace),
            0x2B => Some(Key::Tab),
            0x2C => Some(Key::Space),
            // 符号键也使用 Unicode
            0x2D => Some(Key::Unicode('-')),
            0x2E => Some(Key::Unicode('=')),
            0x2F => Some(Key::Unicode('[')),
            0x30 => Some(Key::Unicode(']')),
            0x31 => Some(Key::Unicode('\\')),
            0x33 => Some(Key::Unicode(';')),
            0x34 => Some(Key::Unicode('\'')),
            0x36 => Some(Key::Unicode(',')),
            0x37 => Some(Key::Unicode('.')),
            0x38 => Some(Key::Unicode('/')),
            0xE0 => Some(Key::Control),
            0xE1 => Some(Key::Shift),
            0xE2 => Some(Key::Alt),
            _ => None,
        }
    }
}

impl InputDriver for SoftwareDriver {
    fn heartbeat(&mut self) {}
    fn switch_identity(&mut self, _index: u8) {}

    fn mouse_abs(&mut self, x: u16, y: u16) {
        // 0.6.1 move_to 接受 Coordinate 枚举
        let _ = self.enigo.move_mouse(x as i32, y as i32, Coordinate::Abs);
    }

    fn mouse_move(&mut self, dx: i32, dy: i32, wheel: i8) {
        let _ = self.enigo.move_mouse(dx, dy, Coordinate::Rel);
        if wheel != 0 {
            // scroll 方法参数可能在不同版本有差异，0.6.1 是 scroll(length, axis)
            // 这里假设纵向滚动
            let _ = self.enigo.scroll(wheel as i32, enigo::Axis::Vertical);
        }
    }

    fn mouse_down(&mut self, left: bool, right: bool) {
        if left { let _ = self.enigo.button(Button::Left, Direction::Press); }
        if right { let _ = self.enigo.button(Button::Right, Direction::Press); }
    }

    fn mouse_up(&mut self) {
        // Enigo 需要显式弹起，这里全部弹起以防万一
        let _ = self.enigo.button(Button::Left, Direction::Release);
        let _ = self.enigo.button(Button::Right, Direction::Release);
    }

    fn key_down(&mut self, keycode: u8, modifier: u8) {
        if (modifier & 0x02) != 0 || (modifier & 0x20) != 0 {
            let _ = self.enigo.key(Key::Shift, Direction::Press);
        }

        if let Some(key) = self.hid_to_enigo(keycode) {
            let _ = self.enigo.key(key, Direction::Press);
            self.last_key = Some(key);
        }
    }

    fn key_up(&mut self) {
        if let Some(key) = self.last_key {
            let _ = self.enigo.key(key, Direction::Release);
            self.last_key = None;
        }
        let _ = self.enigo.key(Key::Shift, Direction::Release);
    }
}

// ==========================================
// 4. 工厂函数
// ==========================================
pub enum DriverType {
    Hardware,
    Software,
}

pub fn create_driver(
    t: DriverType, 
    port: &str, 
    screen_w: u16, 
    screen_h: u16
) -> Result<Box<dyn InputDriver>, String> {
    match t {
        DriverType::Hardware => {
            let drv = HardwareDriver::new(port, 115200, screen_w, screen_h)?;
            Ok(Box::new(drv))
        }
        DriverType::Software => {
            let drv = SoftwareDriver::new(screen_w, screen_h);
            Ok(Box::new(drv))
        }
    }
}
use byteorder::{LittleEndian, WriteBytesExt};
use serialport::SerialPort;
use std::io::Write;
use std::thread;
use std::time::Duration;

// ==========================================
// 私有协议定义 (对外隐藏)
// ==========================================
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

// ==========================================
// 硬件驱动层
// ==========================================
pub struct InputDevice {
    port: Box<dyn SerialPort>,
    pub screen_w: u16,
    pub screen_h: u16,
}

impl InputDevice {
    /// 初始化硬件连接
    pub fn new(port_name: &str, baud_rate: u32, screen_w: u16, screen_h: u16) -> Result<Self, String> {
        let port = serialport::new(port_name, baud_rate)
            .timeout(Duration::from_millis(100))
            .open()
            .map_err(|e| format!("无法打开串口 {}: {}", port_name, e))?;

        Ok(Self { port, screen_w, screen_h })
    }

    /// 核心发送逻辑 (私有)
    fn send_raw(&mut self, event_type: EventType, b: [u8; 6], delay_ms: u16) {
        let mut frame = Vec::with_capacity(11);
        frame.push(FRAME_HEAD);
        frame.push(event_type as u8);
        frame.extend_from_slice(&b);
        frame.write_u16::<LittleEndian>(delay_ms).unwrap();
        frame.push(FRAME_TAIL);

        let _ = self.port.write_all(&frame);
        let _ = self.port.flush();
        // 关键延迟：给 ESP32 和 Windows 处理 HID 帧的时间
        thread::sleep(Duration::from_millis(4)); 
    }

    // ==========================================
    // 公开控制接口 (Public API)
    // ==========================================

    pub fn heartbeat(&mut self) {
        let mut b = [0u8; 6];
        b[0] = SystemCmd::Heartbeat as u8;
        self.send_raw(EventType::System, b, 0);
    }

    pub fn switch_identity(&mut self, index: u8) {
        let mut b = [0u8; 6];
        b[0] = SystemCmd::SetId as u8;
        b[1] = index;
        self.send_raw(EventType::System, b, 0);
    }

    pub fn mouse_abs(&mut self, x: u16, y: u16) {
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

    pub fn mouse_move(&mut self, dx: i32, dy: i32, wheel: i8) {
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

    pub fn mouse_down(&mut self, left: bool, right: bool) {
        let mut mask = 0;
        if left { mask |= 0x01; }
        if right { mask |= 0x02; }
        self.send_raw(EventType::MouseRel, [mask, 0, 0, 0, 0, 0], 0);
    }

    pub fn mouse_up(&mut self) {
        self.send_raw(EventType::MouseRel, [0, 0, 0, 0, 0, 0], 0);
    }

    pub fn key_down(&mut self, keycode: u8, modifier: u8) {
        self.send_raw(EventType::Keyboard, [keycode, 0x00, modifier, 0, 0, 0], 0);
    }

    pub fn key_up(&mut self) {
        self.send_raw(EventType::Keyboard, [0, 0x80, 0, 0, 0, 0], 0);
    }
}
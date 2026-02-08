use byteorder::{LittleEndian, WriteBytesExt};
// ✨ Added Axis to imports
use enigo::{
    Direction, Enigo, Key, Keyboard, Mouse, Settings, Coordinate,
    Button, Axis 
};
use serialport::SerialPort;
use std::io::Write;
use std::thread;
use std::time::Duration;

// ==========================================
// 1. Common Interface (Trait)
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
// 2. Hardware Driver (Serial Port)
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
// 3. Software Driver (Software / Enigo 0.6.1)
// ==========================================
pub struct SoftwareDriver {
    enigo: Enigo,
    pub screen_w: u16,
    pub screen_h: u16,
    last_key: Option<Key>,
}

unsafe impl Sync for SoftwareDriver {}

impl SoftwareDriver {
    pub fn new(screen_w: u16, screen_h: u16) -> Self {
        Self {
            enigo: Enigo::new(&Settings::default()).unwrap(),
            screen_w,
            screen_h,
            last_key: None,
        }
    }

    fn hid_to_enigo(&self, hid: u8) -> Option<Key> {
        match hid {
            0x04..=0x1D => { 
                let c = (b'a' + (hid - 0x04)) as char;
                Some(Key::Unicode(c)) 
            },
            0x1E..=0x27 => { 
                let c = if hid == 0x27 { '0' } else { (b'1' + (hid - 0x1E)) as char };
                Some(Key::Unicode(c))
            },
            0x28 => Some(Key::Return),
            0x29 => Some(Key::Escape),
            0x2A => Some(Key::Backspace),
            0x2B => Some(Key::Tab),
            0x2C => Some(Key::Space),
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
        let _ = self.enigo.move_mouse(x as i32, y as i32, Coordinate::Abs);
    }

    fn mouse_move(&mut self, dx: i32, dy: i32, wheel: i8) {
        let _ = self.enigo.move_mouse(dx, dy, Coordinate::Rel);
        if wheel != 0 {
            // ✨ Corrected scroll usage
            let _ = self.enigo.scroll(-wheel as i32, Axis::Vertical);
        }
    }

    fn mouse_down(&mut self, left: bool, right: bool) {
        if left { let _ = self.enigo.button(Button::Left, Direction::Press); }
        if right { let _ = self.enigo.button(Button::Right, Direction::Press); }
    }

    fn mouse_up(&mut self) {
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
// 4. Factory Function
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
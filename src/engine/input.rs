// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 输入事件（鼠标 / 触摸 / 键盘 / 滚轮）。

use serde::{Deserialize, Serialize};

/// 修饰键。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Modifiers {
    pub alt: bool,
    pub ctrl: bool,
    pub shift: bool,
    pub meta: bool,
}

impl Modifiers {
    pub fn none() -> Self {
        Modifiers::default()
    }
    pub fn with_ctrl(mut self) -> Self {
        self.ctrl = true;
        self
    }
    pub fn with_shift(mut self) -> Self {
        self.shift = true;
        self
    }
    pub fn with_alt(mut self) -> Self {
        self.alt = true;
        self
    }
}

/// 鼠标按键。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MouseButton {
    None,
    Left,
    Middle,
    Right,
}

/// 鼠标事件类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MouseKind {
    Move,
    Down,
    Up,
    Click,
    DoubleClick,
}

/// 鼠标事件。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct MouseEvent {
    pub kind: MouseKind,
    pub x: f64,
    pub y: f64,
    pub button: MouseButton,
    pub modifiers: Modifiers,
    pub click_count: u8,
}

impl MouseEvent {
    pub fn click(x: f64, y: f64, button: MouseButton) -> Self {
        MouseEvent {
            kind: MouseKind::Click,
            x,
            y,
            button,
            modifiers: Modifiers::none(),
            click_count: 1,
        }
    }
    pub fn move_to(x: f64, y: f64) -> Self {
        MouseEvent {
            kind: MouseKind::Move,
            x,
            y,
            button: MouseButton::None,
            modifiers: Modifiers::none(),
            click_count: 0,
        }
    }
    pub fn double_click(x: f64, y: f64) -> Self {
        MouseEvent {
            kind: MouseKind::DoubleClick,
            x,
            y,
            button: MouseButton::Left,
            modifiers: Modifiers::none(),
            click_count: 2,
        }
    }
}

/// 触摸点。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TouchPoint {
    pub id: u32,
    pub x: f64,
    pub y: f64,
}

/// 触摸事件类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TouchKind {
    Start,
    Move,
    End,
    Cancel,
    /// 滑动（含 dx/dy 向量）。
    Swipe,
}

/// 触摸事件。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TouchEvent {
    pub kind: TouchKind,
    pub points: Vec<TouchPoint>,
    pub dx: f64,
    pub dy: f64,
}

impl TouchEvent {
    pub fn tap(x: f64, y: f64) -> Self {
        TouchEvent {
            kind: TouchKind::Start,
            points: vec![TouchPoint { id: 0, x, y }],
            dx: 0.0,
            dy: 0.0,
        }
    }
    pub fn swipe(from: (f64, f64), to: (f64, f64)) -> Self {
        TouchEvent {
            kind: TouchKind::Swipe,
            points: vec![
                TouchPoint {
                    id: 0,
                    x: from.0,
                    y: from.1,
                },
                TouchPoint {
                    id: 0,
                    x: to.0,
                    y: to.1,
                },
            ],
            dx: to.0 - from.0,
            dy: to.1 - from.1,
        }
    }
}

/// 键盘事件类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyKind {
    Down,
    Up,
    /// 字符输入。
    Press,
}

/// 键盘事件。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyEvent {
    pub kind: KeyKind,
    /// 键名，如 "Enter"、"a"、"ArrowDown"。
    pub key: String,
    /// 物理键码，如 "Enter"、"KeyA"。
    pub code: String,
    pub modifiers: Modifiers,
    /// 输入文本（Press 时有效）。
    pub text: String,
}

impl KeyEvent {
    pub fn press(key: impl Into<String>) -> Self {
        let key = key.into();
        KeyEvent {
            kind: KeyKind::Press,
            code: key.clone(),
            text: key.clone(),
            modifiers: Modifiers::none(),
            key,
        }
    }
    pub fn down(key: impl Into<String>) -> Self {
        let key = key.into();
        KeyEvent {
            kind: KeyKind::Down,
            code: key.clone(),
            text: String::new(),
            modifiers: Modifiers::none(),
            key,
        }
    }
}

/// 滚轮事件。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct WheelEvent {
    pub x: f64,
    pub y: f64,
    pub delta_x: f64,
    pub delta_y: f64,
}

/// 统一的输入事件。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InputEvent {
    Mouse(MouseEvent),
    Touch(TouchEvent),
    Key(KeyEvent),
    Wheel(WheelEvent),
}

impl From<MouseEvent> for InputEvent {
    fn from(e: MouseEvent) -> Self {
        InputEvent::Mouse(e)
    }
}
impl From<TouchEvent> for InputEvent {
    fn from(e: TouchEvent) -> Self {
        InputEvent::Touch(e)
    }
}
impl From<KeyEvent> for InputEvent {
    fn from(e: KeyEvent) -> Self {
        InputEvent::Key(e)
    }
}
impl From<WheelEvent> for InputEvent {
    fn from(e: WheelEvent) -> Self {
        InputEvent::Wheel(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_click_defaults() {
        let m = MouseEvent::click(1.0, 2.0, MouseButton::Left);
        assert_eq!(m.kind, MouseKind::Click);
        assert_eq!(m.click_count, 1);
        assert!(!m.modifiers.ctrl);
    }

    #[test]
    fn swipe_vector() {
        let s = TouchEvent::swipe((0.0, 0.0), (100.0, 50.0));
        assert_eq!(s.dx, 100.0);
        assert_eq!(s.dy, 50.0);
    }

    #[test]
    fn from_conversions() {
        let _: InputEvent = MouseEvent::click(0.0, 0.0, MouseButton::Left).into();
        let _: InputEvent = TouchEvent::tap(0.0, 0.0).into();
        let _: InputEvent = KeyEvent::press("Enter").into();
    }
}

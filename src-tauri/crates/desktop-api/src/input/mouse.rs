//! 鼠标控制 — Win32 原生 / macOS CoreGraphics / Linux enigo

use crate::core::*;

/// 移动鼠标到指定坐标
pub async fn move_to(x: i32, y: i32) -> Result<()> {
    #[cfg(windows)]
    {
        use ::windows::Win32::Foundation::POINT;
        use ::windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, SetCursorPos};
        unsafe {
            // SetCursorPos 失败时返回 Err（如系统拦截 / 会话限制）——必须检查，不能静默吞掉
            if SetCursorPos(x, y).is_err() {
                return Err(DesktopError::InputFailed(format!(
                    "SetCursorPos({x}, {y}) 被系统拒绝，鼠标未移动"
                )));
            }
            // 移动后自校验：读取实际光标位置，确认到达目标（容差 2px 防 DPI 舍入）
            let mut pt = POINT::default();
            GetCursorPos(&mut pt).map_err(|e| DesktopError::InputFailed(e.to_string()))?;
            if !pointer_reached(Point { x: pt.x, y: pt.y }, Point { x, y }) {
                return Err(DesktopError::InputFailed(format!(
                    "鼠标移动校验失败：目标({x}, {y})，实际({}, {})——请检查坐标换算/屏幕 DPI/会话限制",
                    pt.x, pt.y
                )));
            }
        }
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        macos::move_to(x, y).await
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        use enigo::{Coordinate, Mouse};
        super::with_enigo(|engine| {
            engine
                .move_mouse(x, y, Coordinate::Abs)
                .map_err(|e| DesktopError::InputFailed(e.to_string()))
        })
    }
    #[cfg(all(not(windows), not(any(target_os = "macos", target_os = "linux"))))]
    {
        Err(DesktopError::PlatformNotSupported)
    }
}

/// 获取鼠标位置
pub async fn position() -> Result<Point> {
    #[cfg(windows)]
    {
        use ::windows::Win32::Foundation::POINT;
        use ::windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
        let mut pt = POINT::default();
        unsafe {
            if GetCursorPos(&mut pt).is_err() {
                return Err(DesktopError::InputFailed("GetCursorPos 失败".to_string()));
            }
        }
        Ok(Point { x: pt.x, y: pt.y })
    }
    #[cfg(target_os = "macos")]
    {
        macos::position()
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        use enigo::Mouse;
        let pos = super::with_enigo(|engine| {
            engine
                .location()
                .map_err(|e| DesktopError::InputFailed(e.to_string()))
        })?;
        Ok(Point { x: pos.0, y: pos.1 })
    }
    #[cfg(all(not(windows), not(any(target_os = "macos", target_os = "linux"))))]
    {
        Err(DesktopError::PlatformNotSupported)
    }
}

/// 点击鼠标
pub async fn click(x: i32, y: i32) -> Result<()> {
    #[cfg(not(target_os = "macos"))]
    move_to(x, y).await?;
    #[cfg(windows)]
    {
        use ::windows::Win32::UI::Input::KeyboardAndMouse::{
            mouse_event, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
        };
        let mut held = super::HeldInput::new(|flags| unsafe { mouse_event(flags, 0, 0, 0, 0) });
        held.hold(MOUSEEVENTF_LEFTUP);
        unsafe {
            mouse_event(MOUSEEVENTF_LEFTDOWN, 0, 0, 0, 0);
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        held.release_last();
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        macos::click_at(x, y, "left", 1).await
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        use enigo::{Button, Direction, Mouse};
        super::with_enigo(|engine| {
            engine
                .button(Button::Left, Direction::Click)
                .map_err(|e| DesktopError::InputFailed(e.to_string()))
        })
    }
    #[cfg(all(not(windows), not(any(target_os = "macos", target_os = "linux"))))]
    {
        Err(DesktopError::PlatformNotSupported)
    }
}

/// 右键点击
pub async fn right_click(x: i32, y: i32) -> Result<()> {
    #[cfg(not(target_os = "macos"))]
    move_to(x, y).await?;
    #[cfg(windows)]
    {
        use ::windows::Win32::UI::Input::KeyboardAndMouse::{
            mouse_event, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
        };
        let mut held = super::HeldInput::new(|flags| unsafe { mouse_event(flags, 0, 0, 0, 0) });
        held.hold(MOUSEEVENTF_RIGHTUP);
        unsafe {
            mouse_event(MOUSEEVENTF_RIGHTDOWN, 0, 0, 0, 0);
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        held.release_last();
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        macos::click_at(x, y, "right", 1).await
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        use enigo::{Button, Direction, Mouse};
        super::with_enigo(|engine| {
            engine
                .button(Button::Right, Direction::Click)
                .map_err(|e| DesktopError::InputFailed(e.to_string()))
        })
    }
    #[cfg(all(not(windows), not(any(target_os = "macos", target_os = "linux"))))]
    {
        Err(DesktopError::PlatformNotSupported)
    }
}

/// Middle button, preserving the same screen-coordinate contract as left/right click.
pub async fn middle_click(x: i32, y: i32) -> Result<()> {
    #[cfg(not(target_os = "macos"))]
    move_to(x, y).await?;
    #[cfg(windows)]
    {
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            mouse_event, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP,
        };
        let mut held = super::HeldInput::new(|flags| unsafe { mouse_event(flags, 0, 0, 0, 0) });
        held.hold(MOUSEEVENTF_MIDDLEUP);
        unsafe {
            mouse_event(MOUSEEVENTF_MIDDLEDOWN, 0, 0, 0, 0);
        }
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        held.release_last();
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        macos::click_at(x, y, "middle", 1).await
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        use enigo::{Button, Direction, Mouse};
        super::with_enigo(|engine| {
            engine
                .button(Button::Middle, Direction::Click)
                .map_err(|error| DesktopError::InputFailed(error.to_string()))
        })
    }
}

/// 滚动鼠标滚轮
///
/// `direction`: "up" / "down" / "left" / "right"；`amount`: 滚轮格数。
pub async fn scroll(direction: &str, amount: i32) -> Result<()> {
    let _ = scroll_delta(direction, amount)?;
    #[cfg(windows)]
    {
        use ::windows::Win32::UI::Input::KeyboardAndMouse::{
            mouse_event, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_WHEEL,
        };
        // Win32 vertical positive means up; horizontal positive means right.
        let delta: i32 = if matches!(direction, "up" | "right") {
            120
        } else {
            -120
        };
        let flags = if matches!(direction, "left" | "right") {
            MOUSEEVENTF_HWHEEL
        } else {
            MOUSEEVENTF_WHEEL
        };
        for _ in 0..amount.max(0) {
            unsafe {
                mouse_event(flags, 0, 0, delta, 0);
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        use enigo::{Axis, Mouse};
        let delta = scroll_delta(direction, amount)?;
        let axis = if matches!(direction, "left" | "right") {
            Axis::Horizontal
        } else {
            Axis::Vertical
        };
        super::with_enigo(|engine| {
            engine
                .scroll(delta, axis)
                .map_err(|e| DesktopError::InputFailed(e.to_string()))
        })
    }
}

/// 拖拽
pub async fn drag(start: Point, end: Point) -> Result<()> {
    #[cfg(not(target_os = "macos"))]
    move_to(start.x, start.y).await?;
    #[cfg(windows)]
    {
        use ::windows::Win32::UI::Input::KeyboardAndMouse::{
            mouse_event, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
        };
        let mut held = super::HeldInput::new(|flags| unsafe { mouse_event(flags, 0, 0, 0, 0) });
        held.hold(MOUSEEVENTF_LEFTUP);
        unsafe {
            mouse_event(MOUSEEVENTF_LEFTDOWN, 0, 0, 0, 0);
        }
        for point in drag_path(start, end, DRAG_STEPS) {
            move_to(point.x, point.y).await?;
            tokio::time::sleep(std::time::Duration::from_millis(DRAG_STEP_MS)).await;
        }
        held.release_last();
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        macos::drag(start, end).await
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        // 拖拽实现：Press → 移动（移动期间不持锁，避免阻塞其他输入）→ Release。
        // 用作用域块让 MutexGuard 在 await 前自然 drop——clippy await_holding_lock
        // 对显式 drop(e) 仍告警（版本差异），作用域块是跨 clippy 版本稳定的写法。
        use enigo::{Button, Direction, Mouse};
        let mut release_error = None;
        let mut held = super::HeldInput::new(|button| {
            if let Err(error) = super::with_enigo(|engine| {
                engine
                    .button(button, Direction::Release)
                    .map_err(|error| DesktopError::InputFailed(error.to_string()))
            }) {
                release_error = Some(error);
            }
        });
        // Register before sending: a native error can follow a successful down.
        held.hold(Button::Left);
        {
            super::with_enigo(|engine| {
                engine
                    .button(Button::Left, Direction::Press)
                    .map_err(|e| DesktopError::InputFailed(e.to_string()))
            })?;
        }
        for point in drag_path(start, end, DRAG_STEPS) {
            move_to(point.x, point.y).await?;
            tokio::time::sleep(std::time::Duration::from_millis(DRAG_STEP_MS)).await;
        }
        held.release_last();
        drop(held);
        match release_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
    #[cfg(all(not(windows), not(any(target_os = "macos", target_os = "linux"))))]
    {
        Err(DesktopError::PlatformNotSupported)
    }
}

// ── 拖拽轨迹 ─────────────────────────────────────────────────────────

/// 拖拽的中间步数（起点与终点之间发射的 Move 事件数）
const DRAG_STEPS: usize = 20;

/// 每步之间的间隔
const DRAG_STEP_MS: u64 = 10;

/// 生成拖拽中间轨迹点（不含起点、含终点）。
///
/// 速度分布取 ease-in-out（三次缓动）：起步与收尾慢、中段快——更接近人工拖拽的
/// 节奏，也让依赖移动事件的接收端（滑块/画布/拖选）拿到连续的中间态，而不是一次
/// 瞬间跳变。轨迹法线方向叠加低幅正弦微抖（两端为 0、中点最大），避免全程严格
/// 共线；抖幅随轨迹长度增长但封顶，长距离拖拽不会更抖。
///
/// 终点为精确值：物理落点必须与调用参数一致。
fn drag_path(start: Point, end: Point, steps: usize) -> Vec<Point> {
    let mut points = Vec::with_capacity(steps);
    if steps == 0 {
        return points;
    }
    let dx = f64::from(end.x - start.x);
    let dy = f64::from(end.y - start.y);
    let len = (dx * dx + dy * dy).sqrt();
    let (nx, ny) = if len > f64::EPSILON {
        (-dy / len, dx / len)
    } else {
        (0.0, 0.0)
    };
    let amp = (len * 0.01).clamp(0.0, 1.5);
    for i in 1..=steps {
        let t = i as f64 / steps as f64;
        // ease-in-out cubic
        let e = if t < 0.5 {
            4.0 * t * t * t
        } else {
            1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
        };
        // sin²(πt) 包络：两端为 0，中途最大
        let envelope = (std::f64::consts::PI * t).sin();
        let wobble = amp * envelope * envelope * (12.0 * std::f64::consts::PI * t).sin();
        points.push(Point {
            x: (f64::from(start.x) + dx * e + nx * wobble).round() as i32,
            y: (f64::from(start.y) + dy * e + ny * wobble).round() as i32,
        });
    }
    // 终点取精确值，避免浮点舍入带来的最后 1px 偏差
    if let Some(last) = points.last_mut() {
        *last = end;
    }
    points
}

// Enigo defines positive vertical lengths as down and negative lengths as up.
fn scroll_delta(direction: &str, amount: i32) -> Result<i32> {
    if amount < 0 {
        return Err(DesktopError::InputFailed(
            "scroll amount must be non-negative".into(),
        ));
    }
    match direction {
        "up" | "left" => Ok(-amount),
        "down" | "right" => Ok(amount),
        _ => Err(DesktopError::InputFailed(format!(
            "unknown scroll direction: {direction}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_direction_matches_native_input_contract() {
        assert_eq!(scroll_delta("up", 3).unwrap(), -3);
        assert_eq!(scroll_delta("down", 3).unwrap(), 3);
        assert_eq!(scroll_delta("left", 3).unwrap(), -3);
        assert_eq!(scroll_delta("right", 3).unwrap(), 3);
        assert_eq!(scroll_delta("down", 0).unwrap(), 0);
        assert!(scroll_delta("up", -1).is_err());
        assert!(scroll_delta("sideways", 3).is_err());
    }

    #[test]
    fn pointer_verification_uses_global_coordinates_including_negative_displays() {
        let requested = Point { x: -1200, y: 400 };
        assert!(pointer_reached(Point { x: -1198, y: 399 }, requested));
        assert!(!pointer_reached(Point { x: -1200, y: 800 }, requested));
        assert!(!pointer_reached(Point { x: 1200, y: 400 }, requested));
        assert!(!pointer_reached(
            Point { x: i32::MIN, y: 0 },
            Point { x: i32::MAX, y: 0 }
        ));
    }

    #[test]
    fn drag_path_keeps_endpoint_and_never_stalls() {
        let start = Point { x: 100, y: 100 };
        let end = Point { x: 500, y: 300 };
        let path = drag_path(start, end, 20);
        assert_eq!(path.len(), 20);
        // 终点必须精确（物理落点不能有浮点舍入偏差）
        assert_eq!(*path.last().unwrap(), end);
        // 单调推进：相邻点不应后退
        for pair in path.windows(2) {
            assert!(pair[1].x >= pair[0].x || pair[1].y >= pair[0].y);
        }
        // 缓动两端速度趋零，末端允许重复点；但有效位移步应占绝大多数
        let moving = path.windows(2).filter(|pair| pair[0] != pair[1]).count()
            + usize::from(path[0] != start);
        assert!(moving >= 15, "moving steps too few: {moving}/20");
    }

    #[test]
    fn drag_path_handles_zero_length_and_single_step() {
        let p = Point { x: 42, y: 24 };
        let path = drag_path(p, p, 20);
        assert_eq!(path.len(), 20);
        // 零长度位移：所有点都落在原地（抖幅为 0，法线未定义时不应产生偏移）
        assert!(path.iter().all(|q| *q == p));

        let one = drag_path(Point { x: 0, y: 0 }, Point { x: 10, y: 0 }, 1);
        assert_eq!(one, vec![Point { x: 10, y: 0 }]);

        assert!(drag_path(Point { x: 0, y: 0 }, Point { x: 10, y: 0 }, 0).is_empty());
    }

    #[test]
    fn drag_path_wobble_stays_bounded() {
        let start = Point { x: 0, y: 0 };
        let end = Point { x: 2000, y: 0 };
        let path = drag_path(start, end, 20);
        // 微抖幅度封顶 1.5px：任意中间点偏离直线（y 方向）不得超过 2px
        for point in &path {
            assert!(point.y.abs() <= 2, "wobble out of bounds: {point:?}");
        }
        // 两端无抖动（端点精确闭合）
        assert_eq!(path.last().unwrap().y, 0);
        if let Some(first) = path.first() {
            assert!(first.y.abs() <= 1);
        }
    }
}

#[cfg(target_os = "macos")]
pub async fn click_at(x: i32, y: i32, button: &str, clicks: i32) -> Result<()> {
    macos::click_at(x, y, button, clicks).await
}

#[cfg(any(windows, target_os = "macos", test))]
fn pointer_reached(actual: Point, expected: Point) -> bool {
    i64::from(actual.x).abs_diff(i64::from(expected.x)) <= 2
        && i64::from(actual.y).abs_diff(i64::from(expected.y)) <= 2
}

// CoreGraphics uses the same top-left global coordinate system as AX and our captures.
// Never re-read Cocoa mouseLocation to synthesize a button event at a different point.
#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use std::ffi::c_void;
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGPoint {
        x: f64,
        y: f64,
    }
    type Event = *const c_void;
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventCreate(source: Event) -> Event;
        fn CGEventGetLocation(event: Event) -> CGPoint;
        fn CGEventCreateMouseEvent(source: Event, kind: u32, point: CGPoint, button: u32) -> Event;
        fn CGEventSetIntegerValueField(event: Event, field: u32, value: i64);
        fn CGEventPost(tap: u32, event: Event);
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(value: Event);
    }
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> u8;
    }
    static INPUT: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    fn check_permission() -> Result<()> {
        if unsafe { AXIsProcessTrusted() } == 0 {
            return Err(DesktopError::InputFailed(
                "请在系统设置→隐私与安全性→辅助功能中授权 Nuphus 后重试".into(),
            ));
        }
        Ok(())
    }
    pub fn position() -> Result<Point> {
        let event = unsafe { CGEventCreate(std::ptr::null()) };
        if event.is_null() {
            return Err(DesktopError::InputFailed("CGEventCreate failed".into()));
        }
        let point = unsafe { CGEventGetLocation(event) };
        unsafe {
            CFRelease(event);
        }
        Ok(Point {
            x: point.x.round() as i32,
            y: point.y.round() as i32,
        })
    }
    fn post(point: Point, kind: u32, button: u32, click_count: i64) -> Result<()> {
        let event = unsafe {
            CGEventCreateMouseEvent(
                std::ptr::null(),
                kind,
                CGPoint {
                    x: point.x as f64,
                    y: point.y as f64,
                },
                button,
            )
        };
        if event.is_null() {
            return Err(DesktopError::InputFailed(
                "CGEventCreateMouseEvent failed".into(),
            ));
        }
        unsafe {
            CGEventSetIntegerValueField(event, 1, click_count);
            CGEventPost(0, event);
            CFRelease(event);
        }
        Ok(())
    }
    async fn verify_position(expected: Point) -> Result<()> {
        for _ in 0..20 {
            let actual = position()?;
            if pointer_reached(actual, expected) {
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let actual = position()?;
        Err(DesktopError::InputFailed(format!(
            "鼠标位置校验失败：请求屏幕坐标 ({}, {})，实际 ({}, {})",
            expected.x, expected.y, actual.x, actual.y
        )))
    }
    pub async fn move_to(x: i32, y: i32) -> Result<()> {
        let _guard = INPUT.lock().await;
        check_permission()?;
        let point = Point { x, y };
        post(point, 5, 0, 0)?;
        verify_position(point).await
    }
    struct Release {
        point: Point,
        kind: u32,
        button: u32,
        count: i64,
        armed: bool,
    }
    impl Drop for Release {
        fn drop(&mut self) {
            if self.armed {
                let _ = post(self.point, self.kind, self.button, self.count);
            }
        }
    }
    pub async fn click_at(x: i32, y: i32, button: &str, clicks: i32) -> Result<()> {
        let (button, down, up) = match button {
            "left" => (0, 1, 2),
            "right" => (1, 3, 4),
            "middle" => (2, 25, 26),
            _ => return Err(DesktopError::InputFailed("unknown mouse button".into())),
        };
        if !(1..=2).contains(&clicks) {
            return Err(DesktopError::InputFailed("clicks must be 1 or 2".into()));
        }
        let _guard = INPUT.lock().await;
        check_permission()?;
        let point = Point { x, y };
        post(point, 5, button, 0)?;
        verify_position(point).await?;
        for count in 1..=clicks {
            check_permission()?;
            let mut release = Release {
                point,
                kind: up,
                button,
                count: count.into(),
                armed: true,
            };
            post(point, down, button, count.into())?;
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            post(point, up, button, count.into())?;
            release.armed = false;
            if count < clicks {
                tokio::time::sleep(std::time::Duration::from_millis(40)).await;
            }
        }
        Ok(())
    }
    pub async fn drag(start: Point, end: Point) -> Result<()> {
        let _guard = INPUT.lock().await;
        check_permission()?;
        post(start, 5, 0, 0)?;
        verify_position(start).await?;
        let mut release = Release {
            point: start,
            kind: 2,
            button: 0,
            count: 1,
            armed: true,
        };
        post(start, 1, 0, 1)?;
        for point in drag_path(start, end, DRAG_STEPS) {
            check_permission()?;
            release.point = point;
            post(point, 6, 0, 1)?;
            tokio::time::sleep(std::time::Duration::from_millis(DRAG_STEP_MS)).await;
        }
        post(end, 2, 0, 1)?;
        release.armed = false;
        verify_position(end).await
    }
}

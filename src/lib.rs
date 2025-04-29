use std::num::NonZeroU32;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use os_terminal::font::TrueTypeFont;
use os_terminal::{DrawTarget, MouseInput, Rgb, Terminal};

use anyhow::Result;
use russh::client;
use russh::{ChannelMsg, Disconnect};
use softbuffer::{NoDisplayHandle, NoWindowHandle, Surface, SurfaceExtWeb};
use tokio::sync::mpsc::{channel, Receiver, Sender};
use wasm_bindgen::prelude::wasm_bindgen;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseScrollDelta, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::platform::web::{EventLoopExtWebSys, WindowAttributesExtWebSys, WindowExtWebSys};
use winit::window::{Window, WindowAttributes, WindowId};
use ws_stream_wasm::WsMeta;

const DISPLAY_SIZE: (usize, usize) = (1024, 768);

#[wasm_bindgen(start)]
fn init() {
    console_error_panic_hook::set_once();
}

#[wasm_bindgen]
pub async fn run(ws_address: String, ssh_username: String, ssh_password: String) {
    let (event_tx, event_rx) = channel(1024);
    let (pty_tx, pty_rx) = channel(1024);

    let display = Display::default();
    let buffer = display.buffer.clone();

    let mut terminal = Terminal::new(display);
    terminal.set_auto_flush(false);
    terminal.set_scroll_speed(5);
    terminal.set_logger(|args| web_log::println!("Terminal: {:?}", args));

    terminal.set_pty_writer(Box::new(move |data| {
        pty_tx.blocking_send(data).unwrap();
    }));

    let font_buffer = include_bytes!("../SourceCodePro.otf");
    terminal.set_font_manager(Box::new(TrueTypeFont::new(10.0, font_buffer)));
    terminal.set_history_size(1000);

    let event_loop = EventLoop::new().unwrap();
    event_loop.set_control_flow(ControlFlow::Wait);

    let terminal = Arc::new(Mutex::new(terminal));
    let pending_draw = Arc::new(AtomicBool::new(true));

    event_loop.spawn_app(App::new(
        event_tx,
        buffer.clone(),
        terminal.clone(),
        pending_draw.clone(),
    ));

    let mut session = Session::connect(
        event_rx,
        pty_rx,
        terminal,
        pending_draw,
        ws_address,
        ssh_username,
        ssh_password,
    )
    .await
    .unwrap();

    let exit_code = session.call().await.unwrap();
    println!("Exitcode: {:?}", exit_code);

    session.close().await.unwrap();
}

#[derive(Debug)]
struct ClientHandler;

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_key: &russh::keys::PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

struct Session {
    session: client::Handle<ClientHandler>,
    event_rx: Receiver<AppEvent>,
    pty_rx: Receiver<String>,
    terminal: Arc<Mutex<Terminal<Display>>>,
    pending_draw: Arc<AtomicBool>,
}

impl Session {
    async fn connect(
        event_rx: Receiver<AppEvent>,
        pty_rx: Receiver<String>,
        terminal: Arc<Mutex<Terminal<Display>>>,
        pending_draw: Arc<AtomicBool>,
        ws_address: String,
        ssh_username: String,
        ssh_password: String,
    ) -> Result<Self> {
        let (_, stream) = WsMeta::connect(ws_address, None).await?;

        let mut session = client::connect_stream(
            Arc::new(client::Config::default()),
            stream.into_io(),
            ClientHandler
        ).await?;

        let auth_ressult = session
            .authenticate_password(ssh_username, ssh_password)
            .await?;

        if !auth_ressult.success() {
            anyhow::bail!("Authentication (with publickey) failed");
        }

        Ok(Self {
            session,
            event_rx,
            pty_rx,
            terminal,
            pending_draw,
        })
    }

    async fn call(&mut self) -> Result<u32> {
        let mut channel = self.session.channel_open_session().await?;

        let (rows, columns) = {
            let term = self.terminal.lock().unwrap();
            (term.rows() as u32, term.columns() as u32)
        };

        channel
            .request_pty(false, "xterm-256color", columns, rows, 0, 0, &[])
            .await?;
        channel.request_shell(true).await?;

        let exit_status = loop {
            tokio::select! {
                Some(event) = self.event_rx.recv() => {
                    let mut terminal = self.terminal.lock().unwrap();
                    match event {
                        AppEvent::Keyboard(byte) => terminal.handle_keyboard(byte),
                        AppEvent::Mouse(mouse) => terminal.handle_mouse(mouse),
                    }
                    drop(terminal);

                    if let Some(pty) = self.pty_rx.try_recv().ok() {
                        channel.data(pty.as_bytes()).await?;
                    } else {
                        self.pending_draw.store(true, Ordering::Relaxed);
                    }
                }
                Some(msg) = channel.wait() => {
                    match msg {
                        ChannelMsg::Data { ref data } => {
                            self.terminal.lock().unwrap().process(data);
                            self.pending_draw.store(true, Ordering::Relaxed);
                        }
                        ChannelMsg::ExitStatus { exit_status } => {
                            break exit_status;
                        }
                        _ => {}
                    }
                },
                Some(pty) = self.pty_rx.recv() => {
                    channel.data(pty.as_bytes()).await?;
                }
            }
        };

        Ok(exit_status)
    }

    async fn close(&mut self) -> Result<()> {
        self.session
            .disconnect(Disconnect::ByApplication, "", "English")
            .await?;
        Ok(())
    }
}

struct Display {
    width: usize,
    height: usize,
    buffer: Rc<Vec<AtomicU32>>,
}

impl Default for Display {
    fn default() -> Self {
        let buffer = (0..DISPLAY_SIZE.0 * DISPLAY_SIZE.1)
            .map(|_| AtomicU32::new(0))
            .collect::<Vec<_>>();

        Self {
            width: DISPLAY_SIZE.0,
            height: DISPLAY_SIZE.1,
            buffer: Rc::new(buffer),
        }
    }
}

impl DrawTarget for Display {
    fn size(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    #[inline(always)]
    fn draw_pixel(&mut self, x: usize, y: usize, color: Rgb) {
        let color = (color.0 as u32) << 16 | (color.1 as u32) << 8 | color.2 as u32;
        self.buffer[y * self.width + x].store(color, Ordering::Relaxed);
    }
}

enum AppEvent {
    Keyboard(u8),
    Mouse(MouseInput),
}

struct App {
    event_tx: Sender<AppEvent>,
    buffer: Rc<Vec<AtomicU32>>,
    terminal: Arc<Mutex<Terminal<Display>>>,
    window: Option<Rc<Window>>,
    surface: Option<Surface<NoDisplayHandle, NoWindowHandle>>,
    pending_draw: Arc<AtomicBool>,
}

impl App {
    fn new(
        event_tx: Sender<AppEvent>,
        buffer: Rc<Vec<AtomicU32>>,
        terminal: Arc<Mutex<Terminal<Display>>>,
        pending_draw: Arc<AtomicBool>,
    ) -> Self {
        Self {
            event_tx,
            buffer,
            terminal,
            window: None,
            surface: None,
            pending_draw,
        }
    }
}

impl ApplicationHandler for App {
    fn new_events(&mut self, _: &ActiveEventLoop, _: StartCause) {
        if !self.pending_draw.swap(false, Ordering::Relaxed) {
            return;
        }

        if let Some(surface) = self.surface.as_mut() {
            let mut surface_buffer = surface.buffer_mut().unwrap();
            self.terminal.lock().unwrap().flush();
            for (index, value) in self.buffer.iter().enumerate() {
                surface_buffer[index] = value.load(Ordering::Relaxed);
            }
            surface_buffer.present().unwrap();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let refresh_rate = event_loop
            .primary_monitor()
            .and_then(|m| m.refresh_rate_millihertz())
            .unwrap_or(60000);
        let frame_duration = 1000.0 / (refresh_rate as f32 / 1000.0);

        use web_time::{Duration, Instant};
        let duration = Duration::from_millis(frame_duration as u64);
        event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + duration));
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let (width, height) = DISPLAY_SIZE;
        let attributes = WindowAttributes::default()
            .with_title("Terminal")
            .with_resizable(false)
            .with_inner_size(PhysicalSize::new(width as f64, height as f64));

        let attributes = WindowAttributesExtWebSys::with_append(attributes, true);

        let window = Rc::new(event_loop.create_window(attributes).unwrap());
        let mut surface = Surface::from_canvas(window.canvas().unwrap()).unwrap();

        surface
            .resize(
                NonZeroU32::new(width as u32).unwrap(),
                NonZeroU32::new(height as u32).unwrap(),
            )
            .unwrap();

        self.window = Some(window);
        self.surface = Some(surface);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, lines) => lines,
                    MouseScrollDelta::PixelDelta(delta) => delta.y as f32,
                };
                let lines = if lines > 0.0 { 1 } else { -1 };
                self.event_tx
                    .blocking_send(AppEvent::Mouse(MouseInput::Scroll(lines)))
                    .unwrap();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let Some(evdev_code) = physicalkey_to_scancode(event.physical_key) {
                    let mut scancode = evdev_code;
                    if event.state == ElementState::Released {
                        scancode += 0x80;
                    }
                    if scancode >= 0xe000 {
                        self.event_tx
                            .blocking_send(AppEvent::Keyboard(0xe0))
                            .unwrap();
                        scancode -= 0xe000;
                    }
                    self.event_tx
                        .blocking_send(AppEvent::Keyboard(scancode as u8))
                        .unwrap();
                }
            }
            _ => {}
        }
    }
}

use winit::keyboard::KeyCode;
use winit::keyboard::PhysicalKey;

// This code is derived from the winit project (https://github.com/rust-windowing/winit),
// originally from src/platform_impl/windows/keyboard.rs.
// Licensed under Apache 2.0 and MIT.
fn physicalkey_to_scancode(physical_key: PhysicalKey) -> Option<u32> {
    let PhysicalKey::Code(code) = physical_key else {
        return None;
    };

    match code {
        KeyCode::Backquote => Some(0x0029),
        KeyCode::Backslash => Some(0x002b),
        KeyCode::Backspace => Some(0x000e),
        KeyCode::BracketLeft => Some(0x001a),
        KeyCode::BracketRight => Some(0x001b),
        KeyCode::Comma => Some(0x0033),
        KeyCode::Digit0 => Some(0x000b),
        KeyCode::Digit1 => Some(0x0002),
        KeyCode::Digit2 => Some(0x0003),
        KeyCode::Digit3 => Some(0x0004),
        KeyCode::Digit4 => Some(0x0005),
        KeyCode::Digit5 => Some(0x0006),
        KeyCode::Digit6 => Some(0x0007),
        KeyCode::Digit7 => Some(0x0008),
        KeyCode::Digit8 => Some(0x0009),
        KeyCode::Digit9 => Some(0x000a),
        KeyCode::Equal => Some(0x000d),
        KeyCode::KeyA => Some(0x001e),
        KeyCode::KeyB => Some(0x0030),
        KeyCode::KeyC => Some(0x002e),
        KeyCode::KeyD => Some(0x0020),
        KeyCode::KeyE => Some(0x0012),
        KeyCode::KeyF => Some(0x0021),
        KeyCode::KeyG => Some(0x0022),
        KeyCode::KeyH => Some(0x0023),
        KeyCode::KeyI => Some(0x0017),
        KeyCode::KeyJ => Some(0x0024),
        KeyCode::KeyK => Some(0x0025),
        KeyCode::KeyL => Some(0x0026),
        KeyCode::KeyM => Some(0x0032),
        KeyCode::KeyN => Some(0x0031),
        KeyCode::KeyO => Some(0x0018),
        KeyCode::KeyP => Some(0x0019),
        KeyCode::KeyQ => Some(0x0010),
        KeyCode::KeyR => Some(0x0013),
        KeyCode::KeyS => Some(0x001f),
        KeyCode::KeyT => Some(0x0014),
        KeyCode::KeyU => Some(0x0016),
        KeyCode::KeyV => Some(0x002f),
        KeyCode::KeyW => Some(0x0011),
        KeyCode::KeyX => Some(0x002d),
        KeyCode::KeyY => Some(0x0015),
        KeyCode::KeyZ => Some(0x002c),
        KeyCode::Minus => Some(0x000c),
        KeyCode::Period => Some(0x0034),
        KeyCode::Quote => Some(0x0028),
        KeyCode::Semicolon => Some(0x0027),
        KeyCode::Slash => Some(0x0035),
        KeyCode::AltLeft => Some(0x0038),
        KeyCode::AltRight => Some(0xe038),
        KeyCode::CapsLock => Some(0x003a),
        KeyCode::ContextMenu => Some(0xe05d),
        KeyCode::ControlLeft => Some(0x001d),
        KeyCode::ControlRight => Some(0xe01d),
        KeyCode::Enter => Some(0x001c),
        KeyCode::ShiftLeft => Some(0x002a),
        KeyCode::ShiftRight => Some(0x0036),
        KeyCode::Space => Some(0x0039),
        KeyCode::Tab => Some(0x000f),
        KeyCode::Convert => Some(0x0079),
        KeyCode::Delete => Some(0xe053),
        KeyCode::End => Some(0xe04f),
        KeyCode::Home => Some(0xe047),
        KeyCode::Insert => Some(0xe052),
        KeyCode::PageDown => Some(0xe051),
        KeyCode::PageUp => Some(0xe049),
        KeyCode::ArrowDown => Some(0xe050),
        KeyCode::ArrowLeft => Some(0xe04b),
        KeyCode::ArrowRight => Some(0xe04d),
        KeyCode::ArrowUp => Some(0xe048),
        KeyCode::NumLock => Some(0xe045),
        KeyCode::Numpad0 => Some(0x0052),
        KeyCode::Numpad1 => Some(0x004f),
        KeyCode::Numpad2 => Some(0x0050),
        KeyCode::Numpad3 => Some(0x0051),
        KeyCode::Numpad4 => Some(0x004b),
        KeyCode::Numpad5 => Some(0x004c),
        KeyCode::Numpad6 => Some(0x004d),
        KeyCode::Numpad7 => Some(0x0047),
        KeyCode::Numpad8 => Some(0x0048),
        KeyCode::Numpad9 => Some(0x0049),
        KeyCode::NumpadAdd => Some(0x004e),
        KeyCode::NumpadComma => Some(0x007e),
        KeyCode::NumpadDecimal => Some(0x0053),
        KeyCode::NumpadDivide => Some(0xe035),
        KeyCode::NumpadEnter => Some(0xe01c),
        KeyCode::NumpadEqual => Some(0x0059),
        KeyCode::NumpadMultiply => Some(0x0037),
        KeyCode::NumpadSubtract => Some(0x004a),
        KeyCode::Escape => Some(0x0001),
        KeyCode::F1 => Some(0x003b),
        KeyCode::F2 => Some(0x003c),
        KeyCode::F3 => Some(0x003d),
        KeyCode::F4 => Some(0x003e),
        KeyCode::F5 => Some(0x003f),
        KeyCode::F6 => Some(0x0040),
        KeyCode::F7 => Some(0x0041),
        KeyCode::F8 => Some(0x0042),
        KeyCode::F9 => Some(0x0043),
        KeyCode::F10 => Some(0x0044),
        KeyCode::F11 => Some(0x0057),
        KeyCode::F12 => Some(0x0058),
        KeyCode::F13 => Some(0x0064),
        KeyCode::F14 => Some(0x0065),
        KeyCode::F15 => Some(0x0066),
        KeyCode::F16 => Some(0x0067),
        KeyCode::F17 => Some(0x0068),
        KeyCode::F18 => Some(0x0069),
        KeyCode::F19 => Some(0x006a),
        KeyCode::F20 => Some(0x006b),
        KeyCode::F21 => Some(0x006c),
        KeyCode::F22 => Some(0x006d),
        KeyCode::F23 => Some(0x006e),
        KeyCode::F24 => Some(0x0076),
        KeyCode::ScrollLock => Some(0x0046),
        KeyCode::Pause => Some(0x0045),
        _ => None,
    }
}

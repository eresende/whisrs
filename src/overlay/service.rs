//! Wayland layer-shell overlay shown while recording or transcribing.

use std::sync::mpsc;
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
enum OverlayError {
    #[error("Wayland connection error: {0}")]
    Connect(#[from] wayland_client::ConnectError),
    #[error("Wayland globals error: {0}")]
    Globals(#[from] wayland_client::globals::GlobalError),
    #[error("smithay bind error: {0}")]
    Bind(#[from] wayland_client::globals::BindError),
    #[error("smithay shm create error: {0}")]
    Shm(#[from] smithay_client_toolkit::shm::CreatePoolError),
    #[error("Wayland dispatch error: {0}")]
    Dispatch(#[from] wayland_client::DispatchError),
    #[error("D-Bus error: {0}")]
    DBus(#[from] zbus::Error),
    #[error("D-Bus signal error: {0}")]
    DBusSignal(#[from] zbus::fdo::Error),
}

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
    shm::{slot::SlotPool, Shm, ShmHandler},
};
use tokio::sync::watch;
use tracing::{info, warn};
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_output, wl_region, wl_shm, wl_surface},
    Connection, Dispatch, QueueHandle,
};

use crate::State;

const WIDTH: u32 = 140;
const HEIGHT: u32 = 28;
const BOTTOM_MARGIN: i32 = 22;
const FADE_STEP: f32 = 0.55;

/// Spawn the bottom recording overlay.
///
/// The Wayland event loop runs on a dedicated OS thread because it is a
/// blocking client loop. A small Tokio task forwards daemon state changes into
/// that thread.
pub async fn spawn_overlay(
    mut state_rx: watch::Receiver<State>,
    mut level_rx: watch::Receiver<f32>,
) {
    let gnome_state_rx = state_rx.clone();
    let gnome_level_rx = level_rx.clone();
    tokio::spawn(async move {
        if let Err(e) = run_gnome_broadcaster(gnome_state_rx, gnome_level_rx).await {
            warn!("GNOME overlay D-Bus broadcaster unavailable: {e:#}");
        }
    });

    let (tx, rx) = mpsc::channel::<State>();
    let (level_tx, level_rx_thread) = mpsc::channel::<f32>();

    std::thread::Builder::new()
        .name("whisrs-overlay".to_string())
        .spawn(move || {
            if let Err(e) = run_overlay(rx, level_rx_thread) {
                warn!("overlay unavailable: {e:#}");
            }
        })
        .map_err(|e| warn!("failed to spawn overlay thread: {e}"))
        .ok();

    tokio::spawn(async move {
        let _ = tx.send(*state_rx.borrow());
        let _ = level_tx.send(*level_rx.borrow());
        loop {
            tokio::select! {
                changed = state_rx.changed() => {
                    if changed.is_err() { break; }
                    if tx.send(*state_rx.borrow()).is_err() { break; }
                }
                changed = level_rx.changed() => {
                    if changed.is_err() { break; }
                    let _ = level_tx.send(*level_rx.borrow());
                }
            }
        }
    });
}

async fn run_gnome_broadcaster(
    mut state_rx: watch::Receiver<State>,
    level_rx: watch::Receiver<f32>,
) -> Result<(), OverlayError> {
    let conn = zbus::connection::Builder::session()?
        .serve_at("/org/whisrs/Overlay", GnomeOverlayBus)?
        .name("org.whisrs.Overlay")?
        .build()
        .await?;

    info!("GNOME overlay D-Bus broadcaster started");
    let initial_state = *state_rx.borrow();
    emit_gnome_state(&conn, initial_state).await?;
    let initial_level = *level_rx.borrow();
    emit_gnome_level(&conn, initial_level).await?;

    // Emit level at a steady ~30 Hz so the GNOME extension always has fresh
    // data, regardless of how the watch channel coalesces updates.
    let mut level_interval = tokio::time::interval(Duration::from_millis(33));
    level_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            changed = state_rx.changed() => {
                if changed.is_err() {
                    break;
                }
                let state = *state_rx.borrow();
                emit_gnome_state(&conn, state).await?;
            }
            _ = level_interval.tick() => {
                let level = *level_rx.borrow();
                emit_gnome_level(&conn, level).await?;
            }
        }
    }

    Ok(())
}

async fn emit_gnome_state(conn: &zbus::Connection, state: State) -> zbus::Result<()> {
    conn.emit_signal(
        None::<&str>,
        "/org/whisrs/Overlay",
        "org.whisrs.Overlay",
        "StateChanged",
        &(state.to_string()),
    )
    .await
}

async fn emit_gnome_level(conn: &zbus::Connection, level: f32) -> zbus::Result<()> {
    conn.emit_signal(
        None::<&str>,
        "/org/whisrs/Overlay",
        "org.whisrs.Overlay",
        "LevelChanged",
        &level.clamp(0.0, 1.0),
    )
    .await
}

struct GnomeOverlayBus;

#[zbus::interface(name = "org.whisrs.Overlay")]
impl GnomeOverlayBus {
    fn ping(&self) -> &'static str {
        "ok"
    }
}

fn run_overlay(
    state_rx: mpsc::Receiver<State>,
    level_rx: mpsc::Receiver<f32>,
) -> Result<(), OverlayError> {
    let conn = Connection::connect_to_env()?;
    let (globals, mut event_queue) = registry_queue_init(&conn)?;
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh)?;
    let layer_shell = LayerShell::bind(&globals, &qh)?;
    let shm = Shm::bind(&globals, &qh)?;

    let surface = compositor.create_surface(&qh);
    let layer =
        layer_shell.create_layer_surface(&qh, surface, Layer::Overlay, Some("whisrs"), None);
    layer.set_anchor(Anchor::BOTTOM);
    layer.set_margin(0, 0, BOTTOM_MARGIN, 0);
    layer.set_exclusive_zone(0);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    layer.set_size(WIDTH, HEIGHT);

    // Make the transparent overlay non-interactive so it never blocks clicks.
    let input_region = compositor.wl_compositor().create_region(&qh, ());
    layer.set_input_region(Some(&input_region));
    input_region.destroy();

    layer.commit();

    let pool = SlotPool::new((WIDTH * HEIGHT * 4) as usize, &shm)?;
    let mut overlay = Overlay {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        shm,
        pool,
        layer,
        state_rx,
        level_rx,
        exit: false,
        first_configure: true,
        width: WIDTH,
        height: HEIGHT,
        target_state: State::Idle,
        visible_state: State::Idle,
        alpha: 0.0,
        frame: 0,
        level: 0.0,
    };

    info!("recording overlay started");
    while !overlay.exit {
        overlay.apply_state_updates();
        event_queue.blocking_dispatch(&mut overlay)?;
    }

    Ok(())
}

struct Overlay {
    registry_state: RegistryState,
    output_state: OutputState,
    shm: Shm,
    pool: SlotPool,
    layer: LayerSurface,
    state_rx: mpsc::Receiver<State>,
    level_rx: mpsc::Receiver<f32>,
    exit: bool,
    first_configure: bool,
    width: u32,
    height: u32,
    target_state: State,
    visible_state: State,
    alpha: f32,
    frame: u32,
    level: f32,
}

impl Overlay {
    fn apply_state_updates(&mut self) {
        while let Ok(state) = self.state_rx.try_recv() {
            self.target_state = state;
            if state != State::Idle {
                self.visible_state = state;
            }
        }
        while let Ok(level) = self.level_rx.try_recv() {
            self.level = level.clamp(0.0, 1.0);
        }
        self.level = (self.level * 0.85).max(0.0);

        let target_alpha = if self.target_state == State::Idle {
            0.0
        } else {
            1.0
        };
        let diff = target_alpha - self.alpha;
        if diff.abs() <= FADE_STEP {
            self.alpha = target_alpha;
        } else {
            self.alpha += diff.signum() * FADE_STEP;
        }
        if self.alpha == 0.0 {
            self.visible_state = State::Idle;
        }
    }

    fn draw(&mut self, qh: &QueueHandle<Self>) {
        self.apply_state_updates();

        let width = self.width;
        let height = self.height;
        let stride = width as i32 * 4;

        let Ok((buffer, canvas)) = self.pool.create_buffer(
            width as i32,
            height as i32,
            stride,
            wl_shm::Format::Argb8888,
        ) else {
            warn!("failed to allocate overlay buffer");
            return;
        };

        draw_overlay(
            canvas,
            width,
            height,
            self.visible_state,
            self.frame,
            self.level,
            self.alpha,
        );
        self.frame = self.frame.wrapping_add(1);

        self.layer
            .wl_surface()
            .damage_buffer(0, 0, width as i32, height as i32);
        self.layer
            .wl_surface()
            .frame(qh, self.layer.wl_surface().clone());
        if let Err(e) = buffer.attach_to(self.layer.wl_surface()) {
            warn!("failed to attach overlay buffer: {e}");
            return;
        }
        self.layer.commit();

        std::thread::sleep(Duration::from_millis(24));
    }
}

impl CompositorHandler for Overlay {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_factor: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
        self.draw(qh);
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for Overlay {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
}

impl LayerShellHandler for Overlay {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _layer: &LayerSurface) {
        self.exit = true;
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        self.width = configure.new_size.0.max(WIDTH);
        self.height = configure.new_size.1.max(HEIGHT);

        if self.first_configure {
            self.first_configure = false;
            self.draw(qh);
        }
    }
}

impl ShmHandler for Overlay {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl Dispatch<wl_region::WlRegion, ()> for Overlay {
    fn event(
        _state: &mut Self,
        _proxy: &wl_region::WlRegion,
        _event: wl_region::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

delegate_compositor!(Overlay);
delegate_output!(Overlay);
delegate_shm!(Overlay);
delegate_layer!(Overlay);
delegate_registry!(Overlay);

impl ProvidesRegistryState for Overlay {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState];
}

fn draw_overlay(
    canvas: &mut [u8],
    width: u32,
    height: u32,
    state: State,
    frame: u32,
    level: f32,
    alpha: f32,
) {
    clear(canvas);

    if alpha <= 0.0 || state == State::Idle {
        return;
    }

    let bg = scale_alpha([235, 18, 20, 24], alpha);
    let accent = match state {
        State::Recording => scale_alpha([255, 239, 68, 68], alpha),
        State::Transcribing => scale_alpha([255, 96, 165, 250], alpha),
        State::Idle => return,
    };

    let radius = height / 2;
    rounded_rect(canvas, width, height, 0, 0, width, height, radius, bg);

    let cy = height / 2;
    let dot_pulse = match state {
        State::Recording => (((frame as f32 / 12.0).sin() + 1.0) * 0.5 * 1.0) as i32,
        _ => 0,
    };
    circle(canvas, width, height, 14, cy, 3 + dot_pulse, accent);

    match state {
        State::Recording => draw_bars(canvas, width, height, accent, frame, level),
        State::Transcribing => draw_sweep(canvas, width, height, accent, frame),
        State::Idle => {}
    }
}

fn clear(canvas: &mut [u8]) {
    canvas.fill(0);
}

#[allow(clippy::too_many_arguments)]
fn rounded_rect(
    canvas: &mut [u8],
    width: u32,
    height: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    radius: u32,
    color: [u8; 4],
) {
    for py in y..y + h {
        for px in x..x + w {
            if inside_rounded_rect(px, py, x, y, w, h, radius) {
                blend_pixel(canvas, width, height, px, py, color);
            }
        }
    }
}

fn inside_rounded_rect(px: u32, py: u32, x: u32, y: u32, w: u32, h: u32, radius: u32) -> bool {
    let right = x + w - 1;
    let bottom = y + h - 1;
    let cx = if px < x + radius {
        x + radius
    } else if px > right.saturating_sub(radius) {
        right.saturating_sub(radius)
    } else {
        px
    };
    let cy = if py < y + radius {
        y + radius
    } else if py > bottom.saturating_sub(radius) {
        bottom.saturating_sub(radius)
    } else {
        py
    };
    let dx = px as i32 - cx as i32;
    let dy = py as i32 - cy as i32;
    dx * dx + dy * dy <= (radius as i32) * (radius as i32)
}

fn scale_alpha(color: [u8; 4], alpha: f32) -> [u8; 4] {
    let a = (color[0] as f32 * alpha.clamp(0.0, 1.0)).round() as u8;
    [a, color[1], color[2], color[3]]
}

const BAR_COUNT: u32 = 5;
const BAR_W: u32 = 6;
const BAR_X_START: u32 = 30;
const BAR_PITCH: u32 = 22;
const BAR_BASELINE: i32 = 3;
const BAR_MAX_H: i32 = 18;

fn draw_bars(canvas: &mut [u8], width: u32, height: u32, color: [u8; 4], frame: u32, level: f32) {
    let cy = (height / 2) as i32;
    for i in 0..BAR_COUNT {
        let variance = 0.7 + (i as f32 * 1.7).sin() * 0.3;
        let phase = ((frame as f32 / 6.0) + i as f32 * 0.9).sin().abs();
        let effective = (level * variance).clamp(0.0, 1.0);
        let dynamic = effective * (0.6 + 0.4 * phase);
        let h = (BAR_BASELINE as f32 + dynamic * (BAR_MAX_H - BAR_BASELINE) as f32)
            .round()
            .max(BAR_BASELINE as f32) as i32;
        let bx = BAR_X_START + i * BAR_PITCH;
        let by = (cy - h / 2).max(0) as u32;
        rounded_rect(canvas, width, height, bx, by, BAR_W, h as u32, 3, color);
    }
}

fn draw_sweep(canvas: &mut [u8], width: u32, height: u32, color: [u8; 4], frame: u32) {
    let cy = (height / 2) as i32;
    let cycle = (BAR_COUNT as i32) * 2 - 2;
    let pos = ((frame / 4) as i32) % cycle.max(1);
    let active = if pos < BAR_COUNT as i32 {
        pos
    } else {
        cycle - pos
    };
    for i in 0..BAR_COUNT {
        let dist = (i as i32 - active).abs() as f32;
        let intensity = (1.0 - dist / 2.5).max(0.18);
        let bar_color = [
            (color[0] as f32 * intensity).round() as u8,
            color[1],
            color[2],
            color[3],
        ];
        let h = (BAR_BASELINE as f32
            + (BAR_MAX_H - BAR_BASELINE) as f32 * (0.45 + 0.55 * intensity))
            .round() as i32;
        let h = h.max(BAR_BASELINE);
        let bx = BAR_X_START + i * BAR_PITCH;
        let by = (cy - h / 2).max(0) as u32;
        rounded_rect(canvas, width, height, bx, by, BAR_W, h as u32, 3, bar_color);
    }
}

fn circle(canvas: &mut [u8], width: u32, height: u32, cx: u32, cy: u32, r: i32, color: [u8; 4]) {
    for y in cy.saturating_sub(r as u32)..=(cy + r as u32).min(height.saturating_sub(1)) {
        for x in cx.saturating_sub(r as u32)..=(cx + r as u32).min(width.saturating_sub(1)) {
            let dx = x as i32 - cx as i32;
            let dy = y as i32 - cy as i32;
            if dx * dx + dy * dy <= r * r {
                blend_pixel(canvas, width, height, x, y, color);
            }
        }
    }
}

fn blend_pixel(canvas: &mut [u8], width: u32, height: u32, x: u32, y: u32, color: [u8; 4]) {
    if x >= width || y >= height {
        return;
    }

    let index = ((y * width + x) * 4) as usize;
    canvas[index..index + 4].copy_from_slice(&u32::from_be_bytes(color).to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_draw_is_transparent() {
        let mut canvas = vec![1; (WIDTH * HEIGHT * 4) as usize];
        draw_overlay(&mut canvas, WIDTH, HEIGHT, State::Idle, 0, 0.0, 0.0);
        assert!(canvas.iter().all(|b| *b == 0));
    }

    #[test]
    fn faded_out_draw_is_transparent() {
        let mut canvas = vec![1; (WIDTH * HEIGHT * 4) as usize];
        draw_overlay(&mut canvas, WIDTH, HEIGHT, State::Recording, 0, 1.0, 0.0);
        assert!(canvas.iter().all(|b| *b == 0));
    }

    #[test]
    fn active_draw_has_visible_pixels() {
        let mut canvas = vec![0; (WIDTH * HEIGHT * 4) as usize];
        draw_overlay(&mut canvas, WIDTH, HEIGHT, State::Recording, 0, 1.0, 1.0);
        assert!(canvas.chunks_exact(4).any(|px| px[3] != 0));
    }

    #[test]
    fn silence_draws_minimal_baseline() {
        // Bar color (red) overdraws the dark background; counting red-dominant
        // pixels measures bar area regardless of the underlying pill.
        // ARGB color is stored on disk as little-endian B,G,R,A — so the
        // canvas byte layout is [B, G, R, A] per pixel.
        fn red_pixels(canvas: &[u8]) -> usize {
            canvas
                .chunks_exact(4)
                .filter(|px| px[2] > 128 && px[0] < 80 && px[1] < 80)
                .count()
        }

        let mut quiet = vec![0; (WIDTH * HEIGHT * 4) as usize];
        let mut loud = vec![0; (WIDTH * HEIGHT * 4) as usize];
        draw_overlay(&mut quiet, WIDTH, HEIGHT, State::Recording, 0, 0.0, 1.0);
        draw_overlay(&mut loud, WIDTH, HEIGHT, State::Recording, 0, 1.0, 1.0);
        let count_quiet = red_pixels(&quiet);
        let count_loud = red_pixels(&loud);
        assert!(
            count_loud > count_quiet,
            "loud audio should fill more bar area than silence (silence={count_quiet}, loud={count_loud})"
        );
    }
}

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

use crate::{OverlayConfig, State};

const BOTTOM_MARGIN: i32 = 16;
const FADE_STEP: f32 = 0.55;

// Bar layout — fixed for visual consistency. 18 bars × 2 px + 17 gaps × 2 px
// = 70 px wide, centered in the pill (15 px side margin at default 100 px).
const BAR_COUNT: u32 = 18;
const BAR_W: u32 = 2;
const BAR_GAP: u32 = 2;
const BAR_PITCH: u32 = BAR_W + BAR_GAP;
const BAR_BLOCK_W: u32 = BAR_COUNT * BAR_W + (BAR_COUNT - 1) * BAR_GAP;
const BAR_BASELINE: i32 = 2;

/// Color palette for one overlay theme. Bytes are stored as `[A, R, G, B]`,
/// matching the canvas pixel layout used by [`blend_pixel`].
#[derive(Debug, Clone, Copy)]
struct Theme {
    bg: [u8; 4],
    ring: [u8; 4],
    rec_bar: [u8; 4],
    trans_bar: [u8; 4],
    glow: [u8; 4],
}

impl Theme {
    /// Default palette — warm "tally light" amber on near-black slate.
    const fn ember() -> Self {
        Self {
            bg: [235, 14, 14, 16],           // #0E0E10 @ 92%
            ring: [64, 249, 115, 22],        // #F97316 @ 25%
            rec_bar: [255, 249, 115, 22],    // #F97316
            trans_bar: [255, 240, 237, 245], // #F0EDF5
            glow: [60, 249, 115, 22],
        }
    }

    /// Monochrome terminal palette — subdued, never distracting.
    const fn carbon() -> Self {
        Self {
            bg: [235, 14, 14, 16],
            ring: [80, 58, 58, 64],          // hairline gray
            rec_bar: [255, 240, 237, 245],   // soft white
            trans_bar: [255, 156, 163, 175], // warm gray
            glow: [40, 240, 237, 245],
        }
    }

    /// Cool electric-blue palette — audio-equipment vibe.
    const fn cyan() -> Self {
        Self {
            bg: [235, 10, 15, 20],
            ring: [64, 34, 211, 238], // #22D3EE @ 25%
            rec_bar: [255, 34, 211, 238],
            trans_bar: [255, 56, 189, 248], // #38BDF8
            glow: [50, 34, 211, 238],
        }
    }

    fn from_config(cfg: &OverlayConfig) -> Self {
        let base = match cfg.theme.as_str() {
            "carbon" => Self::carbon(),
            "cyan" => Self::cyan(),
            "ember" | "custom" => Self::ember(),
            other => {
                warn!("unknown overlay theme {other:?}, falling back to ember");
                Self::ember()
            }
        };
        if cfg.theme != "custom" {
            return base;
        }
        let Some(c) = cfg.colors.as_ref() else {
            return base;
        };
        Self {
            bg: c
                .background
                .as_deref()
                .and_then(crate::parse_hex_color)
                .unwrap_or(base.bg),
            ring: c
                .ring
                .as_deref()
                .and_then(crate::parse_hex_color)
                .unwrap_or(base.ring),
            rec_bar: c
                .recording
                .as_deref()
                .and_then(crate::parse_hex_color)
                .unwrap_or(base.rec_bar),
            trans_bar: c
                .transcribing
                .as_deref()
                .and_then(crate::parse_hex_color)
                .unwrap_or(base.trans_bar),
            glow: c
                .glow
                .as_deref()
                .and_then(crate::parse_hex_color)
                .unwrap_or(base.glow),
        }
    }
}

/// Spawn the bottom recording overlay.
///
/// The Wayland event loop runs on a dedicated OS thread because it is a
/// blocking client loop. A small Tokio task forwards daemon state changes into
/// that thread.
pub async fn spawn_overlay(
    mut state_rx: watch::Receiver<State>,
    mut level_rx: watch::Receiver<f32>,
    config: OverlayConfig,
) {
    let gnome_state_rx = state_rx.clone();
    let gnome_level_rx = level_rx.clone();
    let gnome_theme = config.theme.clone();
    tokio::spawn(async move {
        if let Err(e) = run_gnome_broadcaster(gnome_state_rx, gnome_level_rx, gnome_theme).await {
            warn!("GNOME overlay D-Bus broadcaster unavailable: {e:#}");
        }
    });

    let (tx, rx) = mpsc::channel::<State>();
    let (level_tx, level_rx_thread) = mpsc::channel::<f32>();

    let overlay_config = config;
    std::thread::Builder::new()
        .name("whisrs-overlay".to_string())
        .spawn(move || {
            if let Err(e) = run_overlay(rx, level_rx_thread, overlay_config) {
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
    theme: String,
) -> Result<(), OverlayError> {
    // Custom themes don't sync over D-Bus for v1 — the GNOME extension only
    // knows the named themes it ships. Fall back to "ember" so the bar
    // colors remain sensible.
    let advertised_theme = match theme.as_str() {
        "carbon" | "cyan" | "ember" => theme.clone(),
        _ => "ember".to_string(),
    };

    let conn = zbus::connection::Builder::session()?
        .serve_at("/org/whisrs/Overlay", GnomeOverlayBus)?
        .name("org.whisrs.Overlay")?
        .build()
        .await?;

    info!("GNOME overlay D-Bus broadcaster started");
    emit_gnome_theme(&conn, &advertised_theme).await?;
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

async fn emit_gnome_theme(conn: &zbus::Connection, theme: &str) -> zbus::Result<()> {
    conn.emit_signal(
        None::<&str>,
        "/org/whisrs/Overlay",
        "org.whisrs.Overlay",
        "ThemeChanged",
        &theme,
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
    config: OverlayConfig,
) -> Result<(), OverlayError> {
    let width = config.clamped_width();
    let height = config.clamped_height();
    let theme = Theme::from_config(&config);

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
    layer.set_size(width, height);

    // Make the transparent overlay non-interactive so it never blocks clicks.
    let input_region = compositor.wl_compositor().create_region(&qh, ());
    layer.set_input_region(Some(&input_region));
    input_region.destroy();

    layer.commit();

    let pool = SlotPool::new((width * height * 4) as usize, &shm)?;
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
        width,
        height,
        target_state: State::Idle,
        visible_state: State::Idle,
        alpha: 0.0,
        frame: 0,
        level: 0.0,
        theme,
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
    theme: Theme,
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
            &self.theme,
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
        self.width = configure.new_size.0.max(self.width);
        self.height = configure.new_size.1.max(self.height);

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

#[allow(clippy::too_many_arguments)]
fn draw_overlay(
    canvas: &mut [u8],
    width: u32,
    height: u32,
    state: State,
    frame: u32,
    level: f32,
    alpha: f32,
    theme: &Theme,
) {
    clear(canvas);

    if alpha <= 0.0 || state == State::Idle {
        return;
    }

    let bg = scale_alpha(theme.bg, alpha);
    let ring = scale_alpha(theme.ring, alpha);

    // Pill background.
    let radius = height / 2;
    rounded_rect(canvas, width, height, 0, 0, width, height, radius, bg);
    // 1 px inner ring: paint a slightly inset rect in the ring color, then
    // re-paint the further-inset interior with the bg color. The result is a
    // thin colored band hugging the pill edge.
    if width > 4 && height > 4 {
        rounded_rect(
            canvas,
            width,
            height,
            1,
            1,
            width - 2,
            height - 2,
            radius.saturating_sub(1).max(1),
            ring,
        );
        rounded_rect(
            canvas,
            width,
            height,
            2,
            2,
            width - 4,
            height - 4,
            radius.saturating_sub(2).max(1),
            bg,
        );
    }

    match state {
        State::Recording => draw_bars(canvas, width, height, theme, frame, level, alpha),
        State::Transcribing => draw_sweep(canvas, width, height, theme, frame, alpha),
        State::Idle => {}
    }
}

/// Gaussian taper across the bar row — center bars draw at ~100 % of their
/// dynamic height, edges fall off to ~37 %. `i` is the bar index, `count`
/// the total bar count.
fn taper_factor(i: u32, count: u32) -> f32 {
    if count <= 1 {
        return 1.0;
    }
    let center = (count as f32 - 1.0) / 2.0;
    let d = (i as f32 - center) / center; // -1..=1
    (-d * d).exp() // exp(-1) ≈ 0.367 at edges
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

/// Vertical padding inside the pill (top + bottom). Bars never reach the
/// pill edge.
const BAR_VPAD: i32 = 5;

/// Recording bars: react to audio level, gaussian taper across the row, soft
/// glow halo behind each bar at higher amplitudes.
fn draw_bars(
    canvas: &mut [u8],
    width: u32,
    height: u32,
    theme: &Theme,
    frame: u32,
    level: f32,
    alpha: f32,
) {
    let cy = (height / 2) as i32;
    let max_h = (height as i32 - BAR_VPAD * 2).max(BAR_BASELINE + 2);
    let bar_x_start = (width.saturating_sub(BAR_BLOCK_W)) / 2;

    for i in 0..BAR_COUNT {
        let taper = taper_factor(i, BAR_COUNT);
        // Per-bar phase keeps movement organic instead of marching in lockstep.
        let phase = ((frame as f32 / 5.0) + i as f32 * 0.7).sin().abs();
        let effective = (level * taper).clamp(0.0, 1.0);
        let dynamic = effective * (0.7 + 0.3 * phase);
        let h = (BAR_BASELINE as f32 + dynamic * (max_h - BAR_BASELINE) as f32)
            .round()
            .max(BAR_BASELINE as f32) as i32;
        let bx = bar_x_start + i * BAR_PITCH;
        let by = (cy - h / 2).max(0) as u32;

        // Glow halo behind the bar — only visible above a small threshold.
        if effective > 0.02 {
            let glow_intensity = (effective * 0.9 + 0.1).clamp(0.0, 1.0);
            let glow_a = (theme.glow[0] as f32 * glow_intensity * alpha).round() as u8;
            let glow_color = [glow_a, theme.glow[1], theme.glow[2], theme.glow[3]];
            let glow_w = BAR_W + 2;
            let glow_h = (h + 2).max(BAR_BASELINE + 2) as u32;
            let glow_x = bx.saturating_sub(1);
            let glow_y = ((cy - glow_h as i32 / 2).max(0)) as u32;
            rounded_rect(
                canvas,
                width,
                height,
                glow_x,
                glow_y,
                glow_w,
                glow_h,
                glow_w / 2,
                glow_color,
            );
        }

        let bar_color = scale_alpha(theme.rec_bar, alpha);
        rounded_rect(
            canvas,
            width,
            height,
            bx,
            by,
            BAR_W,
            h as u32,
            BAR_W / 2,
            bar_color,
        );
    }
}

/// Transcribing state: no audio level, just a center-out shimmer that travels
/// across the bar row to communicate "working on it" without flat staticness.
fn draw_sweep(canvas: &mut [u8], width: u32, height: u32, theme: &Theme, frame: u32, alpha: f32) {
    let cy = (height / 2) as i32;
    let max_h = (height as i32 - BAR_VPAD * 2).max(BAR_BASELINE + 2);
    let bar_x_start = (width.saturating_sub(BAR_BLOCK_W)) / 2;

    // Sliding focus point that pings back and forth across the row.
    let cycle = (BAR_COUNT as i32) * 2 - 2;
    let pos = ((frame / 3) as i32) % cycle.max(1);
    let active = if pos < BAR_COUNT as i32 {
        pos as f32
    } else {
        (cycle - pos) as f32
    };

    for i in 0..BAR_COUNT {
        let taper = taper_factor(i, BAR_COUNT);
        let dist = (i as f32 - active).abs();
        // Bell-shaped intensity centered on `active`, ~3 bars wide.
        let intensity = (-dist * dist / 4.0).exp().max(0.15);
        let dynamic = intensity * taper;
        let h = (BAR_BASELINE as f32 + dynamic * (max_h - BAR_BASELINE) as f32 * 0.85)
            .round()
            .max(BAR_BASELINE as f32) as i32;
        let bx = bar_x_start + i * BAR_PITCH;
        let by = (cy - h / 2).max(0) as u32;

        let bar_a = (theme.trans_bar[0] as f32 * (0.3 + 0.7 * intensity) * alpha).round() as u8;
        let bar_color = [
            bar_a,
            theme.trans_bar[1],
            theme.trans_bar[2],
            theme.trans_bar[3],
        ];
        rounded_rect(
            canvas,
            width,
            height,
            bx,
            by,
            BAR_W,
            h as u32,
            BAR_W / 2,
            bar_color,
        );
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

    const W: u32 = 100;
    const H: u32 = 34;

    #[test]
    fn idle_draw_is_transparent() {
        let mut canvas = vec![1; (W * H * 4) as usize];
        let t = Theme::ember();
        draw_overlay(&mut canvas, W, H, State::Idle, 0, 0.0, 0.0, &t);
        assert!(canvas.iter().all(|b| *b == 0));
    }

    #[test]
    fn faded_out_draw_is_transparent() {
        let mut canvas = vec![1; (W * H * 4) as usize];
        let t = Theme::ember();
        draw_overlay(&mut canvas, W, H, State::Recording, 0, 1.0, 0.0, &t);
        assert!(canvas.iter().all(|b| *b == 0));
    }

    #[test]
    fn active_draw_has_visible_pixels() {
        let mut canvas = vec![0; (W * H * 4) as usize];
        let t = Theme::ember();
        draw_overlay(&mut canvas, W, H, State::Recording, 0, 1.0, 1.0, &t);
        assert!(canvas.chunks_exact(4).any(|px| px[3] != 0));
    }

    #[test]
    fn taper_is_strongest_in_center() {
        let center = taper_factor(BAR_COUNT / 2, BAR_COUNT);
        let edge_left = taper_factor(0, BAR_COUNT);
        let edge_right = taper_factor(BAR_COUNT - 1, BAR_COUNT);
        assert!(center > edge_left);
        assert!(center > edge_right);
        assert!(edge_left < 0.5);
        assert!(edge_right < 0.5);
    }

    #[test]
    fn silence_draws_minimal_baseline() {
        // Recording bars in the ember theme are amber (#F97316); count
        // amber-dominant pixels to measure bar area independent of the bg
        // pill. ARGB on disk is little-endian B,G,R,A — each pixel is
        // [B, G, R, A].
        fn amber_pixels(canvas: &[u8]) -> usize {
            canvas
                .chunks_exact(4)
                .filter(|px| px[2] > 220 && px[1] > 80 && px[1] < 180 && px[0] < 60)
                .count()
        }

        let t = Theme::ember();
        let mut quiet = vec![0; (W * H * 4) as usize];
        let mut loud = vec![0; (W * H * 4) as usize];
        draw_overlay(&mut quiet, W, H, State::Recording, 0, 0.0, 1.0, &t);
        draw_overlay(&mut loud, W, H, State::Recording, 0, 1.0, 1.0, &t);
        let count_quiet = amber_pixels(&quiet);
        let count_loud = amber_pixels(&loud);
        assert!(
            count_loud > count_quiet,
            "loud audio should fill more bar area than silence (silence={count_quiet}, loud={count_loud})"
        );
    }
}

//! avizo-service: owns `org.danb.avizo.service` on the session bus and draws
//! the on-screen display (a layer-shell window per output) when asked to.

use std::cell::{Cell, RefCell};
use std::f64::consts::PI;
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk::prelude::*;
use gtk::{cairo, gdk, gdk_pixbuf, gio, glib};
use gtk_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

const BUS_NAME: &str = "org.danb.avizo.service";
const OBJECT_PATH: &str = "/org/danb/avizo/service";
const INTERFACE: &str = "org.danb.avizo.service";
const RESOURCE_PREFIX: &str = "/org/danb/avizo/data/images";

const INTROSPECTION: &str = r#"<node>
  <interface name="org.danb.avizo.service">
    <method name="Show">
      <arg type="d" name="seconds" direction="in"/>
    </method>
    <property type="s" name="ImagePath" access="readwrite"/>
    <property type="s" name="ImageResource" access="readwrite"/>
    <property type="d" name="ImageOpacity" access="readwrite"/>
    <property type="d" name="Progress" access="readwrite"/>
    <property type="i" name="Width" access="readwrite"/>
    <property type="i" name="Height" access="readwrite"/>
    <property type="i" name="Padding" access="readwrite"/>
    <property type="d" name="YOffset" access="readwrite"/>
    <property type="d" name="XOffset" access="readwrite"/>
    <property type="i" name="BorderRadius" access="readwrite"/>
    <property type="i" name="BorderWidth" access="readwrite"/>
    <property type="i" name="BlockHeight" access="readwrite"/>
    <property type="i" name="BlockSpacing" access="readwrite"/>
    <property type="i" name="BlockCount" access="readwrite"/>
    <property type="d" name="FadeIn" access="readwrite"/>
    <property type="d" name="FadeOut" access="readwrite"/>
    <property type="(dddd)" name="Background" access="readwrite"/>
    <property type="(dddd)" name="BorderColor" access="readwrite"/>
    <property type="(dddd)" name="BarFgColor" access="readwrite"/>
    <property type="(dddd)" name="BarBgColor" access="readwrite"/>
    <property type="i" name="Monitor" access="readwrite"/>
    <property type="b" name="FocusedOutput" access="readwrite"/>
  </interface>
</node>"#;

// ------------------------------------------------------------------ helpers

/// A colour as four doubles: red, green, blue and alpha.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rgba {
    red: f64,
    green: f64,
    blue: f64,
    alpha: f64,
}

impl Rgba {
    fn from_bytes(red: i32, green: i32, blue: i32, alpha: f64) -> Rgba {
        Rgba {
            red: red as f64 / 255.0,
            green: green as f64 / 255.0,
            blue: blue as f64 / 255.0,
            alpha,
        }
    }

    fn variant(self) -> glib::Variant {
        (self.red, self.green, self.blue, self.alpha).to_variant()
    }

    fn from_variant(value: &glib::Variant) -> Option<Rgba> {
        let (red, green, blue, alpha) = value.get::<(f64, f64, f64, f64)>()?;
        Some(Rgba {
            red,
            green,
            blue,
            alpha,
        })
    }

    fn set_source(self, ctx: &cairo::Context) {
        ctx.set_source_rgba(self.red, self.green, self.blue, self.alpha);
    }
}

/// C's `(int) double` conversion as x86-64 performs it: out-of-range and NaN
/// values become `INT_MIN`.
fn c_int(value: f64) -> i32 {
    if value.is_nan() || value >= 2_147_483_648.0 || value < -2_147_483_648.0 {
        i32::MIN
    } else {
        value as i32
    }
}

/// `(int) lround(value)`: round half away from zero, then truncate the long
/// to an int. A value lround cannot represent yields `LONG_MIN`, whose low
/// 32 bits are 0.
fn lround_int(value: f64) -> i32 {
    if value.is_nan()
        || value >= 9_223_372_036_854_775_808.0
        || value < -9_223_372_036_854_775_808.0
    {
        0
    } else {
        value.round() as i64 as i32
    }
}

/// Prints a warning the way GLib's default log handler does.
fn warning(message: &str) {
    let now = glib::DateTime::now_local()
        .ok()
        .and_then(|t| {
            let ms = t.microsecond() / 1000;
            t.format("%H:%M:%S").ok().map(|s| format!("{s}.{ms:03}"))
        })
        .unwrap_or_default();
    let program = glib::prgname()
        .map(|p| p.to_string())
        .unwrap_or_else(|| "avizo-service".to_string());
    eprint!(
        "\n** ({program}:{}): WARNING **: {now}: {message}\n",
        std::process::id()
    );
}

fn is_wayland(display: &gdk::Display) -> bool {
    glib::Type::from_name("GdkWaylandDisplay")
        .map(|wayland| display.type_().is_a(wayland))
        .unwrap_or(false)
}

// --------------------------------------------------------------- properties

/// The display settings shared by the service and every window it draws:
/// a window always draws with the service's current values.
#[derive(Clone, Debug)]
struct Props {
    image_opacity: f64,
    progress: f64,
    width: i32,
    height: i32,
    padding: i32,
    y_offset: f64,
    x_offset: f64,
    border_radius: i32,
    border_width: i32,
    block_height: i32,
    block_spacing: i32,
    block_count: i32,
    fade_in: f64,
    fade_out: f64,
    background: Rgba,
    border_color: Rgba,
    bar_fg_color: Rgba,
    bar_bg_color: Rgba,
    monitor: i32,
    focused_output: bool,
}

impl Default for Props {
    fn default() -> Props {
        Props {
            image_opacity: 1.0,
            progress: 0.0,
            width: 248,
            height: 232,
            padding: 24,
            y_offset: 0.75,
            x_offset: 0.50,
            border_radius: 16,
            border_width: 1,
            block_height: 10,
            block_spacing: 2,
            block_count: 20,
            fade_in: 0.2,
            fade_out: 0.5,
            background: Rgba::from_bytes(160, 160, 160, 0.8),
            border_color: Rgba::from_bytes(90, 90, 90, 0.8),
            bar_fg_color: Rgba::from_bytes(0, 0, 0, 0.8),
            bar_bg_color: Rgba::from_bytes(106, 106, 106, 0.8),
            monitor: -1,
            focused_output: false,
        }
    }
}

// ------------------------------------------------------------------- window

/// One rectangle of pixels, as found by `content_box`.
#[derive(Clone, Copy, Debug)]
struct Rect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

/// The smallest rectangle holding every pixel that is not fully transparent.
fn content_box(pixbuf: &gdk_pixbuf::Pixbuf) -> Rect {
    let mut rect = Rect {
        x: 0,
        y: 0,
        width: pixbuf.width(),
        height: pixbuf.height(),
    };
    if !pixbuf.has_alpha() || pixbuf.n_channels() != 4 {
        return rect;
    }
    let bytes = pixbuf.read_pixel_bytes();
    let pixels: &[u8] = &bytes;
    let stride = pixbuf.rowstride() as usize;
    let (width, height) = (pixbuf.width(), pixbuf.height());
    let (mut left, mut right, mut top, mut bottom) = (width, -1, height, -1);
    for y in 0..height {
        for x in 0..width {
            let index = y as usize * stride + x as usize * 4 + 3;
            if pixels.get(index).copied().unwrap_or(0) != 0 {
                left = left.min(x);
                right = right.max(x);
                top = top.min(y);
                bottom = bottom.max(y);
            }
        }
    }
    if right >= 0 {
        rect = Rect {
            x: left,
            y: top,
            width: right - left + 1,
            height: bottom - top + 1,
        };
    }
    rect
}

/// Renders the SVG source of a built-in image so that it lines up with the
/// PNG at `scale` times its size: the PNGs are renders of the SVGs cropped to
/// different margins, so the drawn (non-transparent) parts are matched.
fn render_like(
    svg_path: &str,
    png: &gdk_pixbuf::Pixbuf,
    scale: i32,
) -> Result<gdk_pixbuf::Pixbuf, glib::Error> {
    let png_box = content_box(png);
    let svg_natural = gdk_pixbuf::Pixbuf::from_resource(svg_path)?;
    let svg_natural_box = content_box(&svg_natural);
    let factor = (png_box.width * scale) as f64 / svg_natural_box.width as f64;

    let svg = gdk_pixbuf::Pixbuf::from_resource_at_scale(
        svg_path,
        (svg_natural.width() as f64 * factor).round() as i32,
        (svg_natural.height() as f64 * factor).round() as i32,
        true,
    )?;
    let svg_box = content_box(&svg);

    let result = gdk_pixbuf::Pixbuf::new(
        gdk_pixbuf::Colorspace::Rgb,
        true,
        8,
        png.width() * scale,
        png.height() * scale,
    )
    .ok_or_else(|| glib::Error::new(glib::FileError::Nomem, "cannot allocate image"))?;
    result.fill(0);
    let x = png_box.x * scale;
    let y = png_box.y * scale;
    let width = svg_box.width.min(result.width() - x);
    let height = svg_box.height.min(result.height() - y);
    svg.copy_area(svg_box.x, svg_box.y, width, height, &result, x, y);
    Ok(result)
}

fn draw_rect(ctx: &cairo::Context, x: f64, y: f64, w: f64, h: f64) {
    ctx.line_to(x, y);
    ctx.line_to(x + w, y);
    ctx.line_to(x + w, y + h);
    ctx.line_to(x, y + h);
    ctx.close_path();
    let _ = ctx.fill();
}

fn draw_round_rect(ctx: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    ctx.move_to(x + r, y);
    ctx.line_to(x + w - r, y);
    ctx.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
    ctx.line_to(x + w, y + h - r);
    ctx.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
    ctx.line_to(x + r, y + h);
    ctx.arc(x + r, y + h - r, r, PI / 2.0, PI);
    ctx.line_to(x, y + r);
    ctx.arc(x + r, y + r, r, PI, 3.0 * PI / 2.0);
    ctx.close_path();
    let _ = ctx.fill();
}

/// Draws the frame and the bar; the image is the window's child widget.
fn draw_osd(props: &Props, ctx: &cairo::Context) {
    let width = props.width;
    let height = props.height;
    let padding = props.padding;
    let border_width = props.border_width;
    let block_count = props.block_count;
    let block_spacing = props.block_spacing;
    let block_height = props.block_height;

    let inner = width
        .wrapping_sub(padding.wrapping_mul(2))
        .wrapping_sub(border_width.wrapping_mul(2));
    let gaps = block_count.wrapping_sub(1).wrapping_mul(block_spacing) as f64;
    let block_width = (inner as f64 - gaps) / block_count as f64;

    let blocks_x = padding.wrapping_add(border_width) as f64;
    let blocks_y = height
        .wrapping_sub(padding)
        .wrapping_sub(border_width)
        .wrapping_sub(block_height) as f64;
    let (w, h) = (width as f64, height as f64);
    let bw = border_width as f64;

    ctx.set_operator(cairo::Operator::Source);
    let _ = ctx.paint();

    ctx.set_operator(cairo::Operator::Clear);
    draw_rect(ctx, 0.0, 0.0, w, h);

    ctx.set_operator(cairo::Operator::Source);
    props.border_color.set_source(ctx);
    draw_round_rect(ctx, 0.0, 0.0, w, h, props.border_radius as f64);

    props.background.set_source(ctx);
    let inner_radius = props.border_radius.wrapping_sub(border_width).max(0);
    draw_round_rect(
        ctx,
        bw,
        bw,
        width.wrapping_sub(border_width.wrapping_mul(2)) as f64,
        height.wrapping_sub(border_width.wrapping_mul(2)) as f64,
        inner_radius as f64,
    );

    props.bar_bg_color.set_source(ctx);
    let step = block_width + block_spacing as f64;
    for i in 0..block_count.max(0) {
        draw_rect(
            ctx,
            blocks_x + step * i as f64,
            blocks_y,
            block_width,
            block_height as f64,
        );
    }

    props.bar_fg_color.set_source(ctx);
    if block_spacing > 0 {
        let filled = c_int(block_count as f64 * props.progress);
        for i in 0..filled.max(0) {
            draw_rect(
                ctx,
                blocks_x + step * i as f64,
                blocks_y,
                block_width,
                block_height as f64,
            );
        }
    } else {
        let bar_width = block_width * block_count as f64 * props.progress;
        draw_rect(ctx, blocks_x, blocks_y, bar_width, block_height as f64);
    }

    ctx.set_operator(cairo::Operator::Over);
}

/// The per-window state that is not a display setting.
struct WindowState {
    image_path: String,
    image_resource: String,
    shown: Option<(gdk_pixbuf::Pixbuf, i32)>,
    opacity: f64,
    is_fade_in: bool,
    prev_frame_time: i64,
    tick: Option<gtk::TickCallbackId>,
    on_focused_output: bool,
    place_timeout: bool,
    monitor_set: bool,
}

/// One on-screen display window.
struct OsdWindow {
    window: gtk::Window,
    image: gtk::Image,
    props: Rc<RefCell<Props>>,
    state: RefCell<WindowState>,
}

impl OsdWindow {
    fn new(props: Rc<RefCell<Props>>) -> Rc<OsdWindow> {
        let window = gtk::Window::new(gtk::WindowType::Popup);
        window.set_visible(false);
        window.set_can_focus(false);
        window.set_resizable(false);
        window.set_app_paintable(true);
        window.set_type_hint(gdk::WindowTypeHint::Notification);
        window.set_title("avizo");

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.set_visible(true);
        content.set_can_focus(false);
        let image = gtk::Image::new();
        image.set_visible(true);
        image.set_can_focus(false);
        content.pack_start(&image, true, true, 0);
        window.add(&content);

        if let Some(screen) = WidgetExt::screen(&window) {
            if let Some(visual) = screen.rgba_visual() {
                if screen.is_composited() {
                    window.set_visual(Some(&visual));
                }
            }
        }

        let osd = Rc::new(OsdWindow {
            window,
            image,
            props,
            state: RefCell::new(WindowState {
                image_path: String::new(),
                image_resource: String::new(),
                shown: None,
                opacity: 0.0,
                is_fade_in: true,
                prev_frame_time: 0,
                tick: None,
                on_focused_output: false,
                place_timeout: false,
                monitor_set: false,
            }),
        });

        let props = osd.props.clone();
        osd.window.connect_draw(move |_, ctx| {
            draw_osd(&props.borrow(), ctx);
            glib::Propagation::Proceed
        });
        let weak = Rc::downgrade(&osd);
        osd.window.connect_scale_factor_notify(move |_| {
            if let Some(osd) = weak.upgrade() {
                osd.update_image();
            }
        });
        osd
    }

    /// Applies the size and image opacity settings, as a property binding
    /// does when a window is created or a setting changes.
    fn apply_size(&self) {
        let (width, height) = {
            let props = self.props.borrow();
            (props.width, props.height)
        };
        self.window.set_size_request(width, height);
    }

    fn apply_image_opacity(&self) {
        let opacity = self.props.borrow().image_opacity;
        self.image.set_opacity(opacity);
    }

    fn set_image_path(&self, value: &str) {
        if !value.is_empty() {
            {
                let mut state = self.state.borrow_mut();
                state.image_path = value.to_string();
                state.image_resource.clear();
            }
            self.update_image();
        }
    }

    fn set_image_resource(&self, value: &str) {
        if !value.is_empty() {
            {
                let mut state = self.state.borrow_mut();
                state.image_resource = value.to_string();
                state.image_path.clear();
            }
            self.update_image();
        }
    }

    /// Loads the image at the output's scale factor, so that it stays sharp
    /// on scaled outputs. A failure leaves the current image in place.
    fn update_image(&self) {
        let scale = self.window.scale_factor().max(1);
        let (path, resource) = {
            let state = self.state.borrow();
            (state.image_path.clone(), state.image_resource.clone())
        };
        let loaded: Result<Option<gdk_pixbuf::Pixbuf>, glib::Error> = if !resource.is_empty() {
            let base = format!("{RESOURCE_PREFIX}/{resource}");
            gdk_pixbuf::Pixbuf::from_resource(&format!("{base}.png")).map(|natural| {
                if scale > 1 {
                    match render_like(&format!("{base}.svg"), &natural, scale) {
                        Ok(pixbuf) => Some(pixbuf),
                        Err(_) => natural.scale_simple(
                            natural.width() * scale,
                            natural.height() * scale,
                            gdk_pixbuf::InterpType::Bilinear,
                        ),
                    }
                } else {
                    Some(natural)
                }
            })
        } else if !path.is_empty() {
            match gdk_pixbuf::Pixbuf::file_info(&path) {
                None => Err(glib::Error::new(
                    glib::FileError::Inval,
                    &format!("unknown image format: {path}"),
                )),
                Some((_, width, height)) => gdk_pixbuf::Pixbuf::from_file_at_scale(
                    &path,
                    width * scale,
                    height * scale,
                    true,
                )
                .map(Some),
            }
        } else {
            Ok(None)
        };

        match loaded {
            Err(error) => warning(&format!("Could not load image: {}", error.message())),
            Ok(Some(pixbuf)) => self.show_pixbuf(pixbuf, scale),
            Ok(None) => {}
        }
    }

    fn show_pixbuf(&self, pixbuf: gdk_pixbuf::Pixbuf, scale: i32) {
        let surface = pixbuf.create_surface(scale, None::<&gdk::Window>);
        self.image.set_from_surface(surface.as_ref());
        self.state.borrow_mut().shown = Some((pixbuf, scale));
    }

    fn init_layer_shell(&self) {
        let window = &self.window;
        window.init_layer_shell();
        window.set_layer(Layer::Overlay);
        window.set_anchor(Edge::Top, true);
        window.set_anchor(Edge::Left, true);
        window.set_namespace("avizo");
        window.set_exclusive_zone(-1);
        window.set_keyboard_mode(KeyboardMode::None);
    }

    fn monitor_set(&self) -> bool {
        self.state.borrow().monitor_set
    }

    /// Takes over what a window that is being replaced shows: its image,
    /// its opacity and the progress of its animation.
    fn adopt(&self, old: &OsdWindow) {
        let shown = {
            let old_state = old.state.borrow();
            let mut state = self.state.borrow_mut();
            state.image_path = old_state.image_path.clone();
            state.image_resource = old_state.image_resource.clone();
            state.opacity = old_state.opacity;
            state.is_fade_in = old_state.is_fade_in;
            state.prev_frame_time = old_state.prev_frame_time;
            old_state.shown.clone()
        };
        if let Some((pixbuf, scale)) = shown {
            self.show_pixbuf(pixbuf, scale);
        }
        self.window.set_opacity(old.window.opacity());
    }

    /// Hides and releases a window that has been replaced.
    fn retire(&self) {
        let tick = self.state.borrow_mut().tick.take();
        if let Some(tick) = tick {
            tick.remove();
        }
        self.state.borrow_mut().on_focused_output = false;
        self.window.hide();
        self.window.close();
    }

    /// Shows the window on the given output (Wayland only).
    fn show_on_monitor(self: &Rc<Self>, monitor: &gdk::Monitor) {
        {
            let mut state = self.state.borrow_mut();
            state.on_focused_output = false;
            state.monitor_set = true;
        }
        self.window.set_monitor(monitor);
        self.place_on(monitor);
        self.show_animated();
    }

    /// Shows the window on the output the compositor picks for a surface
    /// that names none, i.e. the focused output (Wayland only). The window
    /// must not have an output set.
    fn show_on_focused_output(self: &Rc<Self>) {
        // A shown surface stays on its output: show it afresh, so that it
        // follows a focus change since the last time.
        if self.window.is_mapped() {
            self.window.hide();
        }
        self.state.borrow_mut().on_focused_output = true;
        self.show_animated();

        // The output is only known once the surface is on it, so the
        // offsets can only be applied then.
        let start = {
            let mut state = self.state.borrow_mut();
            let start = !state.place_timeout;
            state.place_timeout = true;
            start
        };
        if start {
            let weak = Rc::downgrade(self);
            glib::timeout_add_local(Duration::from_millis(20), move || {
                let Some(osd) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                let focused = osd.state.borrow().on_focused_output;
                let gdk_window = osd.window.window();
                match gdk_window {
                    Some(gdk_window) if focused && osd.window.is_visible() => {
                        if let Some(monitor) = osd.window.display().monitor_at_window(&gdk_window) {
                            osd.place_on(&monitor);
                        }
                        glib::ControlFlow::Continue
                    }
                    _ => {
                        osd.state.borrow_mut().place_timeout = false;
                        glib::ControlFlow::Break
                    }
                }
            });
        }
    }

    fn place_on(&self, monitor: &gdk::Monitor) {
        let workarea = monitor.workarea();
        let (top, left) = {
            let props = self.props.borrow();
            (
                lround_int(workarea.height().wrapping_sub(props.height) as f64 * props.y_offset),
                lround_int(workarea.width().wrapping_sub(props.width) as f64 * props.x_offset),
            )
        };
        if self.window.layer_shell_margin(Edge::Top) != top {
            self.window.set_layer_shell_margin(Edge::Top, top);
        }
        if self.window.layer_shell_margin(Edge::Left) != left {
            self.window.set_layer_shell_margin(Edge::Left, left);
        }
    }

    /// Shows the window at the position an X11 window manager allows.
    fn show_on_x11_monitor(self: &Rc<Self>, monitor: &gdk::Monitor) {
        let workarea = monitor.workarea();
        let (top, left) = {
            let props = self.props.borrow();
            (
                lround_int(workarea.height().wrapping_sub(props.height) as f64 * props.y_offset),
                lround_int(workarea.width().wrapping_sub(props.width) as f64 * props.x_offset),
            )
        };
        self.window.set_position(gtk::WindowPosition::Center);
        let _ = self.window.position();
        self.window.move_(left, top);
        self.window.set_type_hint(gdk::WindowTypeHint::Notification);
        self.window.set_accept_focus(false);
        self.show_animated();
    }

    fn start_animation(self: &Rc<Self>, fade_in: bool) {
        let previous = {
            let mut state = self.state.borrow_mut();
            state.is_fade_in = fade_in;
            state.tick.take()
        };
        if let Some(previous) = previous {
            previous.remove();
        }
        let weak: Weak<OsdWindow> = Rc::downgrade(self);
        let tick = self
            .window
            .add_tick_callback(move |_, clock| match weak.upgrade() {
                Some(osd) => osd.animation_tick(clock),
                None => glib::ControlFlow::Break,
            });
        self.state.borrow_mut().tick = Some(tick);
    }

    fn show_animated(self: &Rc<Self>) {
        self.start_animation(true);
        self.window.show();
    }

    fn hide_animated(self: &Rc<Self>) {
        self.start_animation(false);
    }

    fn animation_tick(&self, clock: &gdk::FrameClock) -> glib::ControlFlow {
        enum Step {
            Stop,
            Hide,
            Opacity(f64),
        }

        clock.begin_updating();
        let frame_time = clock.frame_time();
        let (fade_in, fade_out) = {
            let props = self.props.borrow();
            (props.fade_in, props.fade_out)
        };
        let step = {
            let mut state = self.state.borrow_mut();
            let elapsed_us = if state.prev_frame_time == 0 {
                0
            } else {
                frame_time.wrapping_sub(state.prev_frame_time)
            };
            state.prev_frame_time = frame_time;
            let elapsed = elapsed_us as f64 / 1_000_000.0;
            if state.is_fade_in {
                if state.opacity >= 1.0 {
                    state.prev_frame_time = 0;
                    Step::Stop
                } else {
                    state.opacity += elapsed / fade_in;
                    if state.opacity > 1.0 {
                        state.opacity = 1.0;
                    }
                    Step::Opacity(state.opacity)
                }
            } else if state.opacity <= 0.0 {
                Step::Hide
            } else {
                state.opacity -= elapsed / fade_out;
                if state.opacity < 0.0 {
                    state.opacity = 0.0;
                }
                Step::Opacity(state.opacity)
            }
        };

        match step {
            Step::Stop => {
                clock.end_updating();
                glib::ControlFlow::Break
            }
            Step::Hide => {
                self.window.hide();
                self.state.borrow_mut().prev_frame_time = 0;
                clock.end_updating();
                glib::ControlFlow::Break
            }
            Step::Opacity(opacity) => {
                self.window.set_opacity(opacity);
                clock.end_updating();
                glib::ControlFlow::Continue
            }
        }
    }
}

// ------------------------------------------------------------------ service

struct Service {
    props: Rc<RefCell<Props>>,
    image_path: String,
    image_resource: String,
    image_from_path: bool,
    /// One slot per output index; slots fill in as outputs are first used.
    windows: Vec<Option<Rc<OsdWindow>>>,
    open_timeouts: Rc<Cell<i32>>,
}

thread_local! {
    static SERVICE: RefCell<Option<Service>> = const { RefCell::new(None) };
}

fn with_service<R>(f: impl FnOnce(&mut Service) -> R) -> Option<R> {
    SERVICE.with(|service| service.borrow_mut().as_mut().map(f))
}

impl Service {
    fn new() -> Service {
        Service {
            props: Rc::new(RefCell::new(Props::default())),
            image_path: String::new(),
            image_resource: "volume_muted".to_string(),
            image_from_path: false,
            windows: Vec::new(),
            open_timeouts: Rc::new(Cell::new(0)),
        }
    }

    fn live_windows(&self) -> Vec<Rc<OsdWindow>> {
        self.windows.iter().flatten().cloned().collect()
    }

    /// Passes the image selected last (path or resource) on to a window.
    fn set_window_image(&self, window: &OsdWindow) {
        if self.image_from_path {
            window.set_image_path(&self.image_path);
        } else {
            window.set_image_resource(&self.image_resource);
        }
    }

    fn update_window_images(&self) {
        for window in self.live_windows() {
            self.set_window_image(&window);
        }
    }

    fn create_window(&self, display: &gdk::Display, with_image: bool) -> Rc<OsdWindow> {
        let window = OsdWindow::new(self.props.clone());
        window.apply_image_opacity();
        window.apply_size();
        if with_image {
            self.set_window_image(&window);
        }
        if is_wayland(display) {
            window.init_layer_shell();
        }
        window
    }

    fn get_property(&self, name: &str) -> glib::Variant {
        let p = self.props.borrow();
        match name {
            "ImagePath" => self.image_path.to_variant(),
            "ImageResource" => self.image_resource.to_variant(),
            "ImageOpacity" => p.image_opacity.to_variant(),
            "Progress" => p.progress.to_variant(),
            "Width" => p.width.to_variant(),
            "Height" => p.height.to_variant(),
            "Padding" => p.padding.to_variant(),
            "YOffset" => p.y_offset.to_variant(),
            "XOffset" => p.x_offset.to_variant(),
            "BorderRadius" => p.border_radius.to_variant(),
            "BorderWidth" => p.border_width.to_variant(),
            "BlockHeight" => p.block_height.to_variant(),
            "BlockSpacing" => p.block_spacing.to_variant(),
            "BlockCount" => p.block_count.to_variant(),
            "FadeIn" => p.fade_in.to_variant(),
            "FadeOut" => p.fade_out.to_variant(),
            "Background" => p.background.variant(),
            "BorderColor" => p.border_color.variant(),
            "BarFgColor" => p.bar_fg_color.variant(),
            "BarBgColor" => p.bar_bg_color.variant(),
            "Monitor" => p.monitor.to_variant(),
            "FocusedOutput" => p.focused_output.to_variant(),
            _ => "".to_variant(),
        }
    }

    /// Stores a property value. GDBus has already checked the name and the
    /// type against the introspection data.
    fn set_property(&mut self, name: &str, value: &glib::Variant) -> bool {
        enum Effect {
            None,
            Images,
            Size,
            ImageOpacity,
        }

        let effect = {
            let mut p = self.props.borrow_mut();
            let double = || value.get::<f64>();
            let int = || value.get::<i32>();
            let set_f = |slot: &mut f64| double().map(|v| *slot = v).is_some();
            let set_i = |slot: &mut i32| int().map(|v| *slot = v).is_some();
            let set_c = |slot: &mut Rgba| Rgba::from_variant(value).map(|v| *slot = v).is_some();
            let (ok, effect) = match name {
                "ImagePath" => match value.get::<String>() {
                    Some(v) => {
                        self.image_from_path = !v.is_empty();
                        self.image_path = v;
                        (true, Effect::Images)
                    }
                    None => (false, Effect::None),
                },
                "ImageResource" => match value.get::<String>() {
                    Some(v) => {
                        if !v.is_empty() {
                            self.image_from_path = false;
                        }
                        self.image_resource = v;
                        (true, Effect::Images)
                    }
                    None => (false, Effect::None),
                },
                "ImageOpacity" => (set_f(&mut p.image_opacity), Effect::ImageOpacity),
                "Progress" => (set_f(&mut p.progress), Effect::None),
                "Width" => (set_i(&mut p.width), Effect::Size),
                "Height" => (set_i(&mut p.height), Effect::Size),
                "Padding" => (set_i(&mut p.padding), Effect::None),
                "YOffset" => (set_f(&mut p.y_offset), Effect::None),
                "XOffset" => (set_f(&mut p.x_offset), Effect::None),
                "BorderRadius" => (set_i(&mut p.border_radius), Effect::None),
                "BorderWidth" => (set_i(&mut p.border_width), Effect::None),
                "BlockHeight" => (set_i(&mut p.block_height), Effect::None),
                "BlockSpacing" => (set_i(&mut p.block_spacing), Effect::None),
                "BlockCount" => (set_i(&mut p.block_count), Effect::None),
                "FadeIn" => (set_f(&mut p.fade_in), Effect::None),
                "FadeOut" => (set_f(&mut p.fade_out), Effect::None),
                "Background" => (set_c(&mut p.background), Effect::None),
                "BorderColor" => (set_c(&mut p.border_color), Effect::None),
                "BarFgColor" => (set_c(&mut p.bar_fg_color), Effect::None),
                "BarBgColor" => (set_c(&mut p.bar_bg_color), Effect::None),
                "Monitor" => (set_i(&mut p.monitor), Effect::None),
                "FocusedOutput" => match value.get::<bool>() {
                    Some(v) => {
                        p.focused_output = v;
                        (true, Effect::None)
                    }
                    None => (false, Effect::None),
                },
                _ => (false, Effect::None),
            };
            if !ok {
                return false;
            }
            effect
        };

        match effect {
            Effect::None => {}
            Effect::Images => self.update_window_images(),
            Effect::Size => {
                for window in self.live_windows() {
                    window.apply_size();
                }
            }
            Effect::ImageOpacity => {
                for window in self.live_windows() {
                    window.apply_image_opacity();
                }
            }
        }
        true
    }

    fn show(&mut self, seconds: f64) {
        let Some(display) = gdk::Display::default() else {
            return;
        };
        let monitors = display.n_monitors();
        if (self.windows.len() as i64) < monitors as i64 {
            self.windows.resize(monitors.max(0) as usize, None);
        }

        let (monitor, focused_output) = {
            let p = self.props.borrow();
            (p.monitor, p.focused_output)
        };
        if monitor >= monitors {
            eprint!("Monitor: {monitor} does not exist\n");
            return;
        }

        // With no monitor chosen, focused-output shows the display only on
        // the focused output: one window that the compositor places.
        let wayland = is_wayland(&display);
        let on_focused_output = monitor < 0 && focused_output && wayland;
        let target = if on_focused_output { 0 } else { monitor };

        for i in 0..monitors {
            if target != -1 && target != i {
                continue;
            }
            let index = i as usize;
            let mut window = match self.windows[index].clone() {
                Some(window) => window,
                None => {
                    let window = self.create_window(&display, true);
                    self.windows.insert(index, Some(window.clone()));
                    window
                }
            };
            if on_focused_output {
                if window.monitor_set() {
                    // A layer surface that names an output cannot be told to
                    // name none again: hand over to a window that never did.
                    let fresh = self.create_window(&display, false);
                    fresh.adopt(&window);
                    window.retire();
                    self.windows[index] = Some(fresh.clone());
                    window = fresh;
                }
                window.show_on_focused_output();
            } else if let Some(gdk_monitor) = display.monitor(i) {
                if wayland {
                    window.show_on_monitor(&gdk_monitor);
                } else {
                    window.show_on_x11_monitor(&gdk_monitor);
                }
            }
            window.window.queue_draw();
        }

        // Hide what an earlier show left on other outputs.
        if target != -1 {
            for (i, window) in self.windows.iter().enumerate() {
                if let Some(window) = window {
                    if i as i64 != target as i64 && window.window.is_visible() {
                        window.hide_animated();
                    }
                }
            }
        }

        let open = self.open_timeouts.clone();
        open.set(open.get() + 1);
        let interval = c_int(seconds * 1000.0) as u32;
        glib::timeout_add_local_once(Duration::from_millis(interval as u64), move || {
            open.set(open.get() - 1);
            if open.get() == 0 {
                let windows = with_service(|service| service.live_windows()).unwrap_or_default();
                for window in windows {
                    if window.window.is_visible() {
                        window.hide_animated();
                    }
                }
            }
        });
    }
}

// --------------------------------------------------------------------- main

fn on_bus_acquired(connection: gio::DBusConnection) {
    let registered = gio::DBusNodeInfo::for_xml(INTROSPECTION)
        .ok()
        .and_then(|node| node.lookup_interface(INTERFACE))
        .and_then(|interface| {
            connection
                .register_object(
                    OBJECT_PATH,
                    &interface,
                    |_, _, _, _, method, parameters, invocation| {
                        if method == "Show" {
                            if let Some((seconds,)) = parameters.get::<(f64,)>() {
                                with_service(|service| service.show(seconds));
                            }
                        }
                        invocation.return_value(None);
                    },
                    |_, _, _, _, property| {
                        with_service(|service| service.get_property(property))
                            .unwrap_or_else(|| "".to_variant())
                    },
                    |_, _, _, _, property, value| {
                        with_service(|service| service.set_property(property, &value))
                            .unwrap_or(false)
                    },
                )
                .ok()
        });
    if registered.is_none() {
        eprint!("Could not register service\n");
    }
}

fn main() {
    if gtk::init().is_err() {
        eprint!("Cannot open display\n");
        std::process::exit(1);
    }
    gio::resources_register_include!("avizo.gresource").expect("built-in resources");

    SERVICE.with(|service| *service.borrow_mut() = Some(Service::new()));

    let _owner = gio::bus_own_name(
        gio::BusType::Session,
        BUS_NAME,
        gio::BusNameOwnerFlags::NONE,
        |connection, _| on_bus_acquired(connection),
        |_, _| {},
        |_, _| {
            // Another instance owns the name (or the bus went away).
            eprint!("Could not acquire name\n");
            std::process::exit(1);
        },
    );

    gtk::main();
}

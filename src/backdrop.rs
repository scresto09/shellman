use std::cell::RefCell;

use gtk::gdk;
use gtk::glib;
use gtk::graphene;
use gtk::gsk;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::log;
use crate::N_;

const MAX_SIZE: f32 = 2048.0;
// a blurred image has no detail finer than the radius: it is blurred at a reduced
// size where the radius is this many pixels, then enlarged when drawn
const WORK_RADIUS: f32 = 5.0;

pub const FITS: &[(&str, &str)] = &[
    ("cover", N_!("Fill")),
    ("contain", N_!("Fit")),
    ("stretch", N_!("Stretch")),
    ("none", N_!("Original Size")),
    ("tile", N_!("Tile")),
];

pub const POSITIONS: &[(&str, &str, f32, f32)] = &[
    ("top-left", N_!("Top Left"), 0.0, 0.0),
    ("top", N_!("Top"), 0.5, 0.0),
    ("top-right", N_!("Top Right"), 1.0, 0.0),
    ("left", N_!("Left"), 0.0, 0.5),
    ("center", N_!("Center"), 0.5, 0.5),
    ("right", N_!("Right"), 1.0, 0.5),
    ("bottom-left", N_!("Bottom Left"), 0.0, 1.0),
    ("bottom", N_!("Bottom"), 0.5, 1.0),
    ("bottom-right", N_!("Bottom Right"), 1.0, 1.0),
];

// the image is laid out over the whole tab area: each terminal draws its own part
pub const AREA: &str = "shellman-tab-area";

pub fn uses_position(fit: &str) -> bool {
    matches!(fit, "cover" | "contain" | "none")
}

/// The texture may be reduced or blurred: width and height are the size to draw it at.
#[derive(Clone)]
pub struct Image {
    texture: gdk::Texture,
    width: f32,
    height: f32,
}

/// What the terminal background shows. opacity: that of the background color, 0 to 1.
#[derive(Clone, Default)]
pub struct Look {
    pub image: Option<Image>,
    pub color: Option<gdk::RGBA>,
    pub opacity: f64,
    pub fit: String,
    pub position: String,
    pub shared: bool,
    /// nothing opaque below: the color is drawn translucent over the image or the desktop
    pub see_through: bool,
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Backdrop {
        pub look: RefCell<Look>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Backdrop {
        const NAME: &'static str = "ShellManBackdrop";
        type Type = super::Backdrop;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Backdrop {}

    impl WidgetImpl for Backdrop {
        fn snapshot(&self, s: &gtk::Snapshot) {
            crate::util::guard("background paint", (), || self.paint(s));
        }
    }

    impl Backdrop {
        fn paint(&self, s: &gtk::Snapshot) {
            let look = self.look.borrow().clone();
            if look.image.is_none() && !look.see_through {
                return;
            }
            let w = self.obj();
            let (ww, wh) = (w.width() as f32, w.height() as f32);
            if ww <= 0.0 || wh <= 0.0 {
                return;
            }
            let full = graphene::Rect::new(0.0, 0.0, ww, wh);
            if !look.see_through {
                if let Some(c) = look.color {
                    s.append_color(&c, &full);
                }
            }
            if let Some(img) = &look.image {
                let alpha = if look.see_through { 1.0 } else { 1.0 - look.opacity };
                draw_image(&w, s, img, &look, &full, alpha);
            }
            if look.see_through {
                if let Some(mut c) = look.color {
                    c.set_alpha(c.alpha() * look.opacity as f32);
                    s.append_color(&c, &full);
                }
            }
        }
    }

    fn draw_image(w: &super::Backdrop, s: &gtk::Snapshot, img: &Image, look: &Look,
                  full: &graphene::Rect, alpha: f64) {
        let (ww, wh) = (full.width(), full.height());
        let area = look.shared.then(|| {
            std::iter::successors(w.parent(), |p| p.parent()).find(|p| p.has_css_class(AREA))
        }).flatten();
        let (ox, oy, aw, ah) = match area.as_ref().and_then(|a| w.compute_bounds(a).map(|b| (a, b))) {
            Some((a, b)) => (b.x(), b.y(), a.width() as f32, a.height() as f32),
            None => (0.0, 0.0, ww, wh),
        };
        let (iw, ih) = (img.width, img.height);
        let fit = look.fit.as_str();
        let (dw, dh) = match fit {
            "contain" => { let k = (aw / iw).min(ah / ih); (iw * k, ih * k) }
            "stretch" => (aw, ah),
            "none" | "tile" => (iw, ih),
            _ => { let k = (aw / iw).max(ah / ih); (iw * k, ih * k) }
        };
        let (ax, ay) = if uses_position(fit) {
            POSITIONS.iter().find(|p| p.0 == look.position).map_or((0.0, 0.0), |p| (p.2, p.3))
        } else {
            (0.0, 0.0)
        };
        let dest = graphene::Rect::new((aw - dw) * ax - ox, (ah - dh) * ay - oy, dw, dh);
        // the opacity goes inside the repeat: around it, it is lost
        s.push_clip(full);
        if fit == "tile" {
            s.push_repeat(full, Some(&dest));
        }
        s.push_opacity(alpha);
        s.append_scaled_texture(&img.texture, gsk::ScalingFilter::Trilinear, &dest);
        s.pop();
        if fit == "tile" {
            s.pop();
        }
        s.pop();
    }
}

glib::wrapper! {
    pub struct Backdrop(ObjectSubclass<imp::Backdrop>) @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for Backdrop {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl Backdrop {
    pub fn set(&self, look: Look) {
        self.imp().look.replace(look);
        self.queue_draw();
    }
}

const CLEAR_CELLS: &str = "shellman-clear-cells";

// VTE's own stylesheet paints the theme background under the cells
fn ensure_css() {
    thread_local! {
        static DONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    if DONE.replace(true) {
        return;
    }
    let Some(display) = gdk::Display::default() else { return };
    let css = gtk::CssProvider::new();
    css.load_from_string(&format!("vte-terminal.{CLEAR_CELLS} {{ background-color: transparent; }}"));
    gtk::style_context_add_provider_for_display(&display, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
}

pub fn clear_cells(terminal: &gtk::Widget, on: bool) {
    if on {
        ensure_css();
        terminal.add_css_class(CLEAR_CELLS);
    } else {
        terminal.remove_css_class(CLEAR_CELLS);
    }
}

type Cached = ((String, i32), Option<Image>);

thread_local! {
    static CACHE: RefCell<Option<Cached>> = const { RefCell::new(None) };
    static SOURCE: RefCell<Option<(String, gdk::Texture)>> = const { RefCell::new(None) };
}

// decoded and reduced to MAX_SIZE once per image; the blurs start from it
fn source(path: &str) -> Option<gdk::Texture> {
    if let Some(t) = SOURCE.with(|c| c.borrow().as_ref().filter(|(p, _)| p == path).map(|(_, t)| t.clone())) {
        return Some(t);
    }
    let t = match gdk::Texture::from_filename(path) {
        Ok(t) => t,
        Err(e) => {
            log!("cannot load background image {path}: {e}");
            return None;
        }
    };
    let (w, h) = (t.width() as f32, t.height() as f32);
    let scale = MAX_SIZE / w.max(h);
    let t = if scale < 1.0 {
        render(&t, (w * scale).round(), (h * scale).round(), 0.0).unwrap_or(t)
    } else {
        t
    };
    SOURCE.with(|c| *c.borrow_mut() = Some((path.to_string(), t.clone())));
    Some(t)
}

pub fn image(path: &str, blur: i32) -> Option<Image> {
    if path.is_empty() {
        return None;
    }
    let key = (path.to_string(), blur);
    if let Some(t) = CACHE.with(|c| c.borrow().as_ref().filter(|(k, _)| *k == key).map(|(_, t)| t.clone())) {
        return t;
    }
    let t = prepare(path, blur);
    CACHE.with(|c| *c.borrow_mut() = Some((key, t.clone())));
    t
}

fn prepare(path: &str, blur: i32) -> Option<Image> {
    let src = source(path)?;
    let (w, h) = (src.width() as f32, src.height() as f32);
    if blur <= 0 {
        return Some(Image { texture: src, width: w, height: h });
    }
    let r = blur as f32;
    let reduce = if r > WORK_RADIUS { WORK_RADIUS / r } else { 1.0 };
    let t = render(&src, (w * reduce).round().max(1.0), (h * reduce).round().max(1.0), r * reduce)?;
    let (width, height) = (t.width() as f32 / reduce, t.height() as f32 / reduce);
    Some(Image { texture: t, width, height })
}

fn render(src: &gdk::Texture, w: f32, h: f32, r: f32) -> Option<gdk::Texture> {
    let s = gtk::Snapshot::new();
    if r > 0.0 {
        s.push_blur(r as f64);
    }
    s.append_scaled_texture(src, gsk::ScalingFilter::Trilinear, &graphene::Rect::new(0.0, 0.0, w, h));
    if r > 0.0 {
        s.pop();
    }
    let node = s.to_node()?;
    // the blur fades the borders: they are cut off
    let inset = if r > 0.0 && w > 4.0 * r && h > 4.0 * r { r } else { 0.0 };
    let viewport = graphene::Rect::new(inset, inset, w - 2.0 * inset, h - 2.0 * inset);
    let renderer = gsk::CairoRenderer::new();
    if let Err(e) = renderer.realize_for_display(&gdk::Display::default()?) {
        log!("cannot prepare the background image: {e}");
        return None;
    }
    let t = renderer.render_texture(&node, Some(&viewport));
    renderer.unrealize();
    Some(t)
}

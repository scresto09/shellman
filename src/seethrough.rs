// The window background is made transparent and repainted by a widget of ours placed
// under the window content, everywhere except over the terminals: only one property
// of Nautilus changes, its own widgets are left alone.

use std::cell::RefCell;

use gtk::gdk;
use gtk::glib;
use gtk::graphene;
use adw::prelude::*;
use gtk::subclass::prelude::*;

use crate::log;
use crate::settings;

const CLASS: &str = "shellman-see-through";
const SWATCH: &str = "shellman-swatch";
const DATA_KEY: &str = "shellman-see-through";
pub const SPLIT: &str = "shellman-split";

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Painter {
        pub hole: glib::WeakRef<gtk::Widget>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Painter {
        const NAME: &'static str = "ShellManPainter";
        type Type = super::Painter;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk::BinLayout>();
        }
    }

    impl ObjectImpl for Painter {
        fn dispose(&self) {
            while let Some(c) = self.obj().first_child() {
                c.unparent();
            }
        }
    }

    impl WidgetImpl for Painter {
        fn snapshot(&self, s: &gtk::Snapshot) {
            crate::util::guard("see-through paint", (), || self.paint(s));
        }
    }

    impl Painter {
        fn paint(&self, s: &gtk::Snapshot) {
            let w = self.obj();
            let Some(swatch) = w.first_child() else { return };
            let (ww, wh) = (w.width() as f32, w.height() as f32);
            let hole = self.hole.upgrade()
                .filter(|h| h.is_visible() && h.is_mapped())
                .and_then(|h| h.compute_bounds(&*w));
            let Some(h) = hole else {
                w.snapshot_child(&swatch, s);
                return;
            };
            let (x0, y0) = (h.x().max(0.0), h.y().max(0.0));
            let (x1, y1) = ((h.x() + h.width()).min(ww), (h.y() + h.height()).min(wh));
            for (x, y, bw, bh) in [(0.0, 0.0, ww, y0), (0.0, y1, ww, wh - y1),
                                   (0.0, y0, x0, y1 - y0), (x1, y0, ww - x1, y1 - y0)] {
                if bw > 0.0 && bh > 0.0 {
                    s.push_clip(&graphene::Rect::new(x, y, bw, bh));
                    w.snapshot_child(&swatch, s);
                    s.pop();
                }
            }
        }
    }

    #[derive(Default)]
    pub struct Watch {
        pub changed: RefCell<Option<Box<dyn Fn()>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Watch {
        const NAME: &'static str = "ShellManWatch";
        type Type = super::Watch;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk::BinLayout>();
        }
    }

    impl ObjectImpl for Watch {
        fn dispose(&self) {
            while let Some(c) = self.obj().first_child() {
                c.unparent();
            }
        }
    }

    impl WidgetImpl for Watch {
        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.parent_size_allocate(width, height, baseline);
            crate::util::guard("see-through watch", (), || if let Some(f) = self.changed.borrow().as_ref() {
                f();
            });
        }
    }
}

glib::wrapper! {
    pub struct Painter(ObjectSubclass<imp::Painter>) @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

glib::wrapper! {
    /// Wraps the terminal area and reports its moves to the painter of the window.
    pub struct Watch(ObjectSubclass<imp::Watch>) @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Watch {
    pub fn new(child: &impl IsA<gtk::Widget>, win: &gtk::Window) -> Watch {
        let w: Watch = glib::Object::new();
        w.set_hexpand(true);
        w.set_vexpand(true);
        child.set_parent(&w);
        child.bind_property("visible", &w, "visible").sync_create().build();
        let win = win.downgrade();
        let redraw = move || {
            if let Some(win) = win.upgrade() {
                crate::util::guard("see-through redraw", (), || redraw(&win));
            }
        };
        let r = redraw.clone();
        w.connect_visible_notify(move |_| r());
        w.imp().changed.replace(Some(Box::new(redraw)));
        w
    }
}

struct Installed {
    overlay: gtk::Overlay,
    painter: Painter,
    content: gtk::Widget,
}

pub fn available() -> bool {
    std::env::var_os("SHELLMAN_TEST_SEE_THROUGH").is_some()
        || gdk::Display::default().is_some_and(|d| d.is_composited() && d.is_rgba())
}

pub fn active() -> bool {
    settings::with(|s| s.see_through) && available()
}

fn ensure_css() {
    thread_local! {
        static DONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    if DONE.replace(true) {
        return;
    }
    let Some(display) = gdk::Display::default() else { return };
    let priority = gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1;
    let base = gtk::CssProvider::new();
    // the separators between split terminals show the window background, like elsewhere
    base.load_from_string(&format!(
        "window.{CLASS} {{ background: none; }}\n.{SWATCH} {{ border-radius: 12px; }}\n\
         window.{CLASS} paned.{SPLIT} > separator {{ background-color: @window_bg_color; }}\n\
         window.view.{CLASS} paned.{SPLIT} > separator {{ background-color: @view_bg_color; }}"));
    gtk::style_context_add_provider_for_display(&display, &base, priority);
    // libadwaita 1.6+ exposes the window radius (0 when maximized); older versions keep 12px
    let radius = gtk::CssProvider::new();
    radius.load_from_string(&format!(
        ".{SWATCH} {{ border-radius: var(--window-radius); }}\n\
         window.{CLASS} paned.{SPLIT} > separator {{ background-color: var(--window-bg-color); }}\n\
         window.view.{CLASS} paned.{SPLIT} > separator {{ background-color: var(--view-bg-color); }}"));
    gtk::style_context_add_provider_for_display(&display, &radius, priority);
}

fn installed(win: &gtk::Window) -> Option<&Installed> {
    unsafe { win.data::<Installed>(DATA_KEY).map(|p| p.as_ref()) }
}

pub fn redraw(win: &gtk::Window) {
    if let Some(i) = installed(win) {
        i.painter.queue_draw();
    }
}

pub fn apply(win: &gtk::Window, hole: &gtk::Widget) {
    let on = active();
    if on == installed(win).is_some() {
        if on {
            sync_classes(win);
        }
        return;
    }
    let Some(aw) = win.downcast_ref::<adw::ApplicationWindow>() else {
        log!("see-through: not an AdwApplicationWindow");
        return;
    };
    let focus = gtk::prelude::GtkWindowExt::focus(win);
    if on {
        let Some(content) = aw.content() else { return };
        ensure_css();
        let painter: Painter = glib::Object::new();
        painter.imp().hole.set(Some(hole));
        painter.set_can_target(false);
        let swatch = gtk::Box::new(gtk::Orientation::Vertical, 0);
        swatch.add_css_class(SWATCH);
        swatch.set_parent(&painter);
        let overlay = gtk::Overlay::new();
        overlay.set_hexpand(true);
        overlay.set_vexpand(true);
        overlay.set_child(Some(&painter));
        aw.set_content(None::<&gtk::Widget>);
        overlay.add_overlay(&content);
        overlay.set_measure_overlay(&content, true);
        aw.set_content(Some(&overlay));
        unsafe { win.set_data(DATA_KEY, Installed { overlay, painter, content }) };
        sync_classes(win);
        win.add_css_class(CLASS);
        log!("see-through enabled");
    } else {
        let Some(i) = (unsafe { win.steal_data::<Installed>(DATA_KEY) }) else { return };
        win.remove_css_class(CLASS);
        aw.set_content(None::<&gtk::Widget>);
        i.overlay.remove_overlay(&i.content);
        aw.set_content(Some(&i.content));
        log!("see-through disabled");
    }
    if let Some(f) = focus.filter(|f| f.root().is_some()) {
        f.grab_focus();
    }
}

// the swatch takes the classes that give the window its background color
fn sync_classes(win: &gtk::Window) {
    let Some(swatch) = installed(win).and_then(|i| i.painter.first_child()) else { return };
    for c in ["background", "view"] {
        if win.has_css_class(c) {
            swatch.add_css_class(c);
        } else {
            swatch.remove_css_class(c);
        }
    }
}

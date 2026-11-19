use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};

use super::Pane;
use crate::i18n::tr;
use crate::settings;
use crate::util::guard;

const MARKS_WIDTH: i32 = 6;
// the jump buttons show up when the pointer is this close to the scrollbar
const JUMP_ZONE: f64 = 64.0;
const MAX_PROMPTS: usize = 2000;
const SETTLE: std::time::Duration = std::time::Duration::from_millis(80);

pub(super) struct Prompts {
    marks: gtk::DrawingArea,
    jump: gtk::Box,
    prev: gtk::Button,
    next: gtk::Button,
    /// scrollback rows of the prompts
    rows: RefCell<Vec<i64>>,
    stamp: Cell<Option<std::time::SystemTime>>,
    settle: RefCell<Option<glib::SourceId>>,
    pub hovered: Cell<bool>,
    fg: Cell<Option<gdk::RGBA>>,
    /// prompt reached with the jump buttons or shortcuts; None means the last one
    selected: Cell<Option<i64>>,
}

impl Prompts {
    pub fn set_fg(&self, fg: gdk::RGBA) {
        self.fg.set(Some(fg));
        self.marks.queue_draw();
    }

    pub fn update_tooltips(&self) {
        for (b, label, action) in [(&self.prev, tr("Previous Prompt"), "prev-prompt"),
                                   (&self.next, tr("Next Prompt"), "next-prompt")] {
            let keys = crate::prefs::accel_text(&settings::accels(action));
            let text = if settings::accels(action).is_empty() { label } else { format!("{label} ({keys})") };
            b.set_tooltip_text(Some(&text));
        }
    }

    pub fn new(layers: &gtk::Overlay) -> Prompts {
        let marks = gtk::DrawingArea::new();
        marks.set_width_request(MARKS_WIDTH);
        marks.set_halign(gtk::Align::End);
        marks.set_can_target(false);
        layers.add_overlay(&marks);
        let jump = gtk::Box::new(gtk::Orientation::Vertical, 2);
        jump.add_css_class("osd");
        jump.set_halign(gtk::Align::End);
        jump.set_valign(gtk::Align::End);
        jump.set_margin_end(MARKS_WIDTH + 6);
        jump.set_margin_bottom(6);
        jump.set_visible(false);
        let prev = gtk::Button::from_icon_name("go-up-symbolic");
        let next = gtk::Button::from_icon_name("go-down-symbolic");
        for b in [&prev, &next] {
            b.add_css_class("flat");
            b.set_focus_on_click(false);
            jump.append(b);
        }
        layers.add_overlay(&jump);
        let p = Prompts {
            marks, jump, prev, next,
            rows: RefCell::new(Vec::new()),
            stamp: Cell::new(None),
            settle: RefCell::new(None),
            hovered: Cell::new(false),
            fg: Cell::new(None),
            selected: Cell::new(None),
        };
        p.update_tooltips();
        p
    }

    pub fn connect(pane: &Rc<Pane>) {
        let p = &pane.prompts;
        let weak = Rc::downgrade(pane);
        p.marks.set_draw_func(move |_, cr, w, h| {
            if let Some(p) = weak.upgrade() {
                guard("prompt marks", (), || p.draw_prompt_marks(cr, w, h));
            }
        });
        for (b, forward) in [(&p.prev, false), (&p.next, true)] {
            let weak = Rc::downgrade(pane);
            b.connect_clicked(move |_| {
                if let Some(p) = weak.upgrade() {
                    guard("prompt jump", (), || p.jump_to_prompt(forward));
                }
            });
        }

        let hover = gtk::EventControllerMotion::new();
        let near_edge = {
            let weak = Rc::downgrade(pane);
            move |c: &gtk::EventControllerMotion, x: f64| {
                let Some(p) = weak.upgrade() else { return };
                guard("prompt hover", (), || {
                    let near = c.widget().is_some_and(|w| x >= w.width() as f64 - JUMP_ZONE);
                    if p.prompts.hovered.replace(near) != near {
                        p.update_prompt_marks();
                    }
                });
            }
        };
        let f = near_edge.clone();
        hover.connect_enter(move |c, x, _| f(c, x));
        hover.connect_motion(move |c, x, _| near_edge(c, x));
        let weak = Rc::downgrade(pane);
        hover.connect_leave(move |_| {
            if let Some(p) = weak.upgrade() {
                guard("prompt hover", (), || if p.prompts.hovered.replace(false) {
                    p.update_prompt_marks();
                });
            }
        });
        pane.layers.add_controller(hover);

        if let Some(adj) = pane.vadjustment() {
            let weak = Rc::downgrade(pane);
            adj.connect_changed(move |_| {
                if let Some(p) = weak.upgrade() {
                    guard("prompt marks", (), || p.update_prompt_marks());
                }
            });
            let weak = Rc::downgrade(pane);
            adj.connect_value_changed(move |a| {
                if let Some(p) = weak.upgrade() {
                    guard("prompt scroll", (), || {
                        let shown = |r: i64| (r as f64) >= a.value() - 0.5 && (r as f64) < a.value() + a.page_size();
                        if p.prompts.selected.get().is_some_and(|r| !shown(r)) {
                            p.prompts.selected.set(None);
                        }
                        p.prompts.marks.queue_draw();
                    });
                }
            });
        }

        let alt_click = gtk::GestureClick::new();
        alt_click.set_button(gdk::BUTTON_PRIMARY);
        alt_click.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(pane);
        alt_click.connect_pressed(move |g, n, x, y| {
            let mods = g.current_event_state() & gtk::accelerator_get_default_mod_mask();
            if n != 1 || mods != gdk::ModifierType::ALT_MASK {
                return;
            }
            if let Some(p) = weak.upgrade() {
                if guard("alt click", false, || p.move_cursor_to(x, y)) {
                    g.set_state(gtk::EventSequenceState::Claimed);
                }
            }
        });
        pane.term.widget().add_controller(alt_click);
    }
}

impl Pane {
    pub(super) fn at_prompt(&self) -> bool {
        self.pid.get().is_some_and(|pid| self.term.shell_idle(pid))
    }

    fn vadjustment(&self) -> Option<gtk::Adjustment> {
        self.term.widget().dynamic_cast_ref::<gtk::Scrollable>().and_then(|s| s.vadjustment())
    }

    fn new_prompt_stamp(&self) -> Option<std::time::SystemTime> {
        if !self.integrated.get() || !self.at_prompt() {
            return None;
        }
        let stamp = std::fs::metadata(&self.state_file).ok()?.modified().ok()?;
        if Some(stamp) == self.prompts.stamp.get() {
            return None;
        }
        std::fs::read(&self.state_file).is_ok_and(|s| s == b"prompt").then_some(stamp)
    }

    // each prompt rewrites the state file: a new modification time means a new prompt.
    // The file is written before the terminal has read the end of the previous output,
    // so the row is taken once the output has settled.
    pub(super) fn note_prompt(self: &Rc<Self>) {
        if self.new_prompt_stamp().is_none() {
            return;
        }
        if let Some(id) = self.prompts.settle.take() {
            id.remove();
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(SETTLE, move || {
            if let Some(p) = weak.upgrade() {
                p.prompts.settle.take();
                guard("prompt row", (), || p.record_prompt());
            }
        });
        *self.prompts.settle.borrow_mut() = Some(id);
    }

    fn record_prompt(&self) {
        let Some(stamp) = self.new_prompt_stamp() else { return };
        self.prompts.stamp.set(Some(stamp));
        let row = self.term.cursor().1;
        let mut rows = self.prompts.rows.borrow_mut();
        if rows.last() != Some(&row) {
            rows.retain(|&r| r < row);
            rows.push(row);
            if rows.len() > MAX_PROMPTS {
                rows.remove(0);
            }
        }
        drop(rows);
        self.update_prompt_marks();
    }

    fn visible_prompts(&self) -> Vec<i64> {
        let Some(adj) = self.vadjustment() else { return Vec::new() };
        let lower = adj.lower().ceil() as i64;
        self.prompts.rows.borrow().iter().copied().filter(|&r| r >= lower).collect()
    }

    pub fn update_prompt_marks(&self) {
        let (marks, buttons) = settings::with(|s| (s.prompt_marks, s.prompt_buttons));
        let scrollable = self.vadjustment().is_some_and(|a| a.upper() - a.lower() > a.page_size() + 0.5);
        let several = (marks || buttons) && scrollable && self.visible_prompts().len() > 1;
        self.prompts.marks.set_visible(several && marks);
        self.prompts.jump.set_visible(several && buttons && self.prompts.hovered.get());
        self.prompts.marks.queue_draw();
    }

    fn draw_prompt_marks(&self, cr: &gtk::cairo::Context, width: i32, height: i32) {
        let Some(adj) = self.vadjustment() else { return };
        let span = adj.upper() - adj.lower();
        if span <= 0.0 {
            return;
        }
        let color = self.prompts.fg.get().unwrap_or_else(|| self.term.widget().color());
        let (r, g, b) = (color.red().into(), color.green().into(), color.blue().into());
        let mark = |row: i64, w: f64, alpha: f64| {
            let h = 2.0;
            let x = width as f64 - w;
            let y = ((row as f64 - adj.lower()) / span * height as f64).clamp(0.0, height as f64 - h).floor();
            cr.set_source_rgba(r, g, b, alpha);
            cr.new_sub_path();
            cr.arc(x + 1.0, y + 1.0, 1.0, std::f64::consts::FRAC_PI_2, 3.0 * std::f64::consts::FRAC_PI_2);
            cr.line_to(x + w, y);
            cr.line_to(x + w, y + h);
            cr.close_path();
            let _ = cr.fill();
        };
        let prompts = self.visible_prompts();
        let Some(&last) = prompts.last() else { return };
        let current = self.prompts.selected.get().filter(|r| prompts.contains(r)).unwrap_or(last);
        for &row in prompts.iter().filter(|&&r| r != current) {
            mark(row, 3.0, 0.35);
        }
        mark(current, MARKS_WIDTH as f64, 0.75);
    }

    pub(super) fn jump_to_prompt(&self, forward: bool) {
        let Some(adj) = self.vadjustment() else { return };
        let prompts = self.visible_prompts();
        let Some(&last) = prompts.last() else { return };
        let bottom = adj.upper() - adj.page_size();
        let from = match self.prompts.selected.get() {
            Some(r) => r as f64,
            None if adj.value() >= bottom - 0.5 => last as f64,
            None => adj.value(),
        };
        let target = if forward {
            prompts.iter().copied().find(|&r| r as f64 > from + 0.5)
        } else {
            match prompts.iter().copied().rev().find(|&r| (r as f64) < from - 0.5) {
                Some(r) => Some(r),
                None => return,
            }
        };
        self.prompts.selected.set(target.filter(|&r| r != last));
        let value = target.map_or(bottom, |r| r as f64);
        adj.set_value(value.clamp(adj.lower(), bottom));
        self.prompts.marks.queue_draw();
    }

    pub fn update_cursor_shape(&self) {
        let bar = settings::with(|s| s.cursor_bar) && self.at_prompt();
        if self.bar_cursor.replace(bar) != bar {
            self.term.set_bar_cursor(bar);
        }
    }

    // arrow keys from the cursor to the clicked cell, counted along the wrapped command line
    fn move_cursor_to(&self, x: f64, y: f64) -> bool {
        if !self.at_prompt() {
            return false;
        }
        let (cw, ch) = (self.term.char_width(), self.term.char_height());
        let Some(vadj) = self.vadjustment() else { return false };
        if cw <= 0 || ch <= 0 {
            return false;
        }
        const PADDING: f64 = 1.0;
        let col = ((x - PADDING) / cw as f64).floor() as i64;
        let row = vadj.value().floor() as i64 + ((y - PADDING) / ch as f64).floor() as i64;
        let (ccol, crow) = self.term.cursor();
        let offset = (row - crow) * self.term.columns() + (col - ccol);
        if offset == 0 || offset.abs() > 10_000 {
            return offset == 0;
        }
        let key = if offset > 0 { "\x1b[C" } else { "\x1b[D" };
        self.term.feed(&key.repeat(offset.unsigned_abs() as usize));
        true
    }
}

// Theme choice in the preferences: a row on the Appearance page and a subpage that
// lists every theme with a preview of its colors.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use crate::colors::{self, Source, Variant, FOLLOW_TERMINAL, NAUTILUS};
use crate::i18n::tr;
use crate::settings;
use crate::util::guard;

struct Entry {
    id: String,
    name: String,
    detail: Option<String>,
}

fn terminal_detail() -> String {
    match colors::default_terminal() {
        Some(t) => tr("Colors of {terminal}").replace("{terminal}", t.name()),
        None => tr("Neither Ptyxis nor GNOME Terminal found: Nautilus colors"),
    }
}

fn sections_list() -> Vec<(String, Vec<Entry>)> {
    let entry = |id: &str, name: String, detail: Option<String>| Entry { id: id.into(), name, detail };
    vec![
        (tr("Automatic"), vec![
            entry(FOLLOW_TERMINAL, tr("Same as My Terminal"), Some(terminal_detail())),
            entry(NAUTILUS, tr("Same as Nautilus"), Some(tr("The background of the file view"))),
        ]),
        (tr("Built-in"), colors::builtin().into_iter().map(|(id, name)| entry(id, name, None)).collect()),
    ].into_iter().chain(Source::ALL.into_iter().map(|source| (
        source.terminal().map_or_else(|| tr("My Themes"), String::from),
        colors::themes(source).into_iter().map(|t| entry(&t.id, t.name, None)).collect(),
    ))).collect()
}

fn name_of(id: &str) -> String {
    match id {
        FOLLOW_TERMINAL => tr("Same as My Terminal"),
        NAUTILUS => tr("Same as Nautilus"),
        _ => match colors::split_id(id) {
            Some((source, _)) => {
                let name = colors::theme_name(id).unwrap_or_else(|| id.to_string());
                match source.terminal() {
                    Some(t) => format!("{name} ({t})"),
                    None => name,
                }
            }
            None => colors::builtin().into_iter().find(|(i, _)| *i == id).map(|(_, n)| n)
                .unwrap_or_else(|| id.to_string()),
        },
    }
}

fn rounded(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    use std::f64::consts::{FRAC_PI_2, PI};
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, FRAC_PI_2, PI);
    cr.arc(x + r, y + r, r, PI, 3.0 * FRAC_PI_2);
    cr.close_path();
}

fn swatch(v: Variant) -> (gtk::DrawingArea, Rc<RefCell<Variant>>) {
    let colors = Rc::new(RefCell::new(v));
    let area = gtk::DrawingArea::new();
    area.set_content_width(64);
    area.set_content_height(40);
    area.set_valign(gtk::Align::Center);
    let c = colors.clone();
    area.set_draw_func(move |a, cr, w, h| guard("theme preview", (), || {
        let v = c.borrow();
        let ink = a.color();
        let set = |c: &gtk::gdk::RGBA, alpha: f64| {
            cr.set_source_rgba(c.red().into(), c.green().into(), c.blue().into(), alpha);
        };
        let (w, h) = (w as f64, h as f64);
        rounded(cr, 0.5, 0.5, w - 1.0, h - 1.0, 6.0);
        if let Some(bg) = &v.bg {
            set(bg, 1.0);
            let _ = cr.fill_preserve();
        }
        set(&ink, 0.15);
        cr.set_line_width(1.0);
        let _ = cr.stroke();
        let fg = v.fg.unwrap_or(ink);
        set(&fg, 1.0);
        rounded(cr, 7.0, 8.0, w * 0.55, 3.0, 1.5);
        rounded(cr, 7.0, 15.0, w * 0.35, 3.0, 1.5);
        let _ = cr.fill();
        if v.palette.len() >= 7 {
            let step = (w - 14.0) / 6.0;
            for (i, c) in v.palette[1..7].iter().enumerate() {
                set(c, 1.0);
                cr.arc(7.0 + step * (i as f64 + 0.5), h - 10.0, 3.0, 0.0, std::f64::consts::TAU);
                let _ = cr.fill();
            }
        }
    }));
    (area, colors)
}

struct Section {
    group: adw::PreferencesGroup,
    button: gtk::ToggleButton,
}

struct Row {
    row: adw::ActionRow,
    check: gtk::Image,
    name: String,
    id: String,
    section: usize,
}

struct Page {
    scrolled: gtk::ScrolledWindow,
    column: gtk::Box,
    search: gtk::SearchEntry,
    sections: Vec<Section>,
    /// index of the My Themes section
    mine: Option<usize>,
    rows: RefCell<Vec<Row>>,
    select: Rc<dyn Fn(&str)>,
}

impl Page {
    fn top_of(&self, w: &impl IsA<gtk::Widget>) -> Option<f64> {
        w.compute_point(&self.column, &gtk::graphene::Point::new(0.0, 0.0)).map(|p| p.y() as f64)
    }

    fn scroll_to(&self, y: f64) {
        let adj = self.scrolled.vadjustment();
        adj.set_value(y.clamp(adj.lower(), adj.upper() - adj.page_size()));
    }

    fn go_to_section(&self, i: usize) {
        if let Some(y) = self.sections.get(i).and_then(|s| self.top_of(&s.group)) {
            self.scroll_to(y - 12.0);
        }
    }

    fn current_row(&self) -> Option<adw::ActionRow> {
        let id = settings::with(|s| s.palette.clone());
        self.rows.borrow().iter().find(|r| r.id == id).map(|r| r.row.clone())
    }

    /// the current theme's row and the scroll value that centers it, once the rows have a place
    fn target(&self) -> Option<(adw::ActionRow, f64)> {
        let row = self.current_row()?;
        let adj = self.scrolled.vadjustment();
        if row.height() <= 0 || adj.page_size() <= 0.0 {
            return None;
        }
        let y = self.top_of(&row)? - (adj.page_size() - row.height() as f64) / 2.0;
        Some((row, y.clamp(adj.lower(), (adj.upper() - adj.page_size()).max(adj.lower()))))
    }

    // Positions are only final once a layout has run with the rows shown: the view is
    // moved on each frame until the target stays the same, then the row gets the focus.
    fn place_on_current(self: &Rc<Self>) {
        let p = Rc::downgrade(self);
        let last = std::cell::Cell::new(None::<f64>);
        let frames = std::cell::Cell::new(0u32);
        self.scrolled.add_tick_callback(move |w, _| {
            let Some(p) = p.upgrade() else { return glib::ControlFlow::Break };
            frames.set(frames.get() + 1);
            let done = guard("current theme", true, || {
                if p.current_row().is_none() {
                    return true;
                }
                let Some((row, y)) = p.target() else { return frames.get() > 60 };
                let stable = last.replace(Some(y)).is_some_and(|l| (l - y).abs() < 0.5);
                p.scrolled.vadjustment().set_value(y);
                if stable || frames.get() > 60 {
                    row.grab_focus();
                    return true;
                }
                false
            });
            if !done {
                return glib::ControlFlow::Continue;
            }
            w.set_opacity(1.0);
            guard("theme section", (), || p.update_section());
            glib::ControlFlow::Break
        });
    }

    fn go_to_current(self: &Rc<Self>) {
        if !self.search.text().is_empty() {
            self.search.set_text("");
        }
        self.place_on_current();
    }

    // the list stays hidden until it is placed on the current theme: no visible scrolling
    fn open_on_current(self: &Rc<Self>) {
        self.scrolled.set_opacity(0.0);
        self.place_on_current();
    }

    fn query(&self) -> String {
        self.search.text().trim().to_lowercase()
    }

    fn update_groups(&self) {
        let rows = self.rows.borrow();
        let searching = !self.query().is_empty();
        for (i, s) in self.sections.iter().enumerate() {
            let any = rows.iter().any(|r| r.section == i && r.row.is_visible())
                || (Some(i) == self.mine && !searching);
            s.group.set_visible(any);
            s.button.set_sensitive(any);
        }
    }

    fn filter(&self) {
        let q = self.query();
        for r in self.rows.borrow().iter() {
            r.row.set_visible(r.name.contains(&q));
        }
        self.update_groups();
    }

    fn add(self: &Rc<Self>, section: usize, e: Entry, current: &str) {
        let row = adw::ActionRow::builder().title(&e.name).use_markup(false).activatable(true).build();
        if let Some(d) = &e.detail {
            row.set_subtitle(d);
        }
        row.add_prefix(&swatch(colors::preview(&e.id)).0);
        let check = gtk::Image::from_icon_name("object-select-symbolic");
        check.set_visible(e.id == current);
        row.add_suffix(&check);
        if let Some((Source::Mine, file)) = colors::split_id(&e.id) {
            let delete = gtk::Button::builder().icon_name("user-trash-symbolic").valign(gtk::Align::Center)
                .tooltip_text(tr("Delete")).css_classes(["flat"]).build();
            let (file, title, weak) = (file.to_string(), e.name.clone(), Rc::downgrade(self));
            delete.connect_clicked(move |b| guard("delete theme", (), || {
                let alert = adw::AlertDialog::new(Some(&tr("Delete “{name}”?").replace("{name}", &title)), None);
                alert.add_response("cancel", &tr("Cancel"));
                alert.add_response("delete", &tr("Delete"));
                alert.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
                alert.set_close_response("cancel");
                let (file, weak) = (file.clone(), weak.clone());
                alert.connect_response(Some("delete"), move |_, _| {
                    if let Some(p) = weak.upgrade() {
                        guard("delete theme", (), || p.delete(&file));
                    }
                });
                alert.present(Some(b));
            }));
            row.add_suffix(&delete);
        }
        let name = e.name.to_lowercase();
        row.set_visible(name.contains(&self.query()));
        let (id, weak) = (e.id.clone(), Rc::downgrade(self));
        row.connect_activated(move |_| {
            if let Some(p) = weak.upgrade() {
                guard("theme", (), || {
                    for r in p.rows.borrow().iter() {
                        r.check.set_visible(r.id == id);
                    }
                    (p.select)(&id);
                });
            }
        });
        self.sections[section].group.add(&row);
        self.rows.borrow_mut().push(Row { row, check, name, id: e.id, section });
    }

    fn toast(&self, msg: &str) {
        if let Some(d) = self.scrolled.ancestor(adw::PreferencesDialog::static_type())
            .and_downcast::<adw::PreferencesDialog>() {
            d.add_toast(adw::Toast::new(msg));
        }
    }

    fn import(self: &Rc<Self>) {
        let Some(mine) = self.mine else { return };
        let window = self.scrolled.root().and_downcast::<gtk::Window>();
        let dialog = gtk::FileDialog::builder().title(tr("Import a Theme")).modal(true).build();
        let weak = Rc::downgrade(self);
        dialog.open(window.as_ref(), gtk::gio::Cancellable::NONE, move |res| {
            let (Some(p), Ok(file)) = (weak.upgrade(), res) else { return };
            guard("import theme", (), || {
                let Some((name, v)) = file.path().and_then(|f| colors::read_theme_file(&f)) else {
                    return p.toast(&tr("Not a theme of Black Box, Ghostty, Ptyxis or Tilix"));
                };
                let name = free_name(&name);
                let Some(id) = colors::save_mine(&name, &v, None) else {
                    return p.toast(&tr("The theme could not be saved"));
                };
                p.add(mine, Entry { id: id.clone(), name, detail: None }, &id);
                for r in p.rows.borrow().iter() {
                    r.check.set_visible(r.id == id);
                }
                p.update_groups();
                (p.select)(&id);
                p.go_to_current();
            });
        });
    }

    fn delete(&self, file: &str) {
        colors::delete_mine(file);
        let id = format!("{}{file}", Source::Mine.prefix());
        let mut rows = self.rows.borrow_mut();
        if let Some(i) = rows.iter().position(|r| r.id == id) {
            let r = rows.remove(i);
            self.sections[r.section].group.remove(&r.row);
        }
        let current = settings::with(|s| s.palette == id);
        if current {
            for r in rows.iter() {
                r.check.set_visible(r.id == FOLLOW_TERMINAL);
            }
        }
        drop(rows);
        self.update_groups();
        if current {
            (self.select)(FOLLOW_TERMINAL);
        }
    }

    // the section at the top of the view, or the last one at the end of the list
    fn update_section(&self) {
        let adj = self.scrolled.vadjustment();
        let visible: Vec<usize> = (0..self.sections.len()).filter(|&i| self.sections[i].group.is_visible()).collect();
        let at_end = adj.value() >= adj.upper() - adj.page_size() - 1.0;
        let active = if at_end {
            visible.last().copied()
        } else {
            visible.iter().copied()
                .filter(|&i| self.top_of(&self.sections[i].group).is_some_and(|y| y <= adj.value() + 24.0))
                .last()
                .or(visible.first().copied())
        };
        if let Some(i) = active {
            if !self.sections[i].button.is_active() {
                self.sections[i].button.set_active(true);
            }
        }
    }
}

fn page(current: &str, select: Rc<dyn Fn(&str)>) -> adw::NavigationPage {
    let search = gtk::SearchEntry::builder()
        .placeholder_text(tr("Search Themes"))
        .hexpand(true)
        .build();
    let tabs = gtk::Box::builder().spacing(4).halign(gtk::Align::Center).build();
    let top = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6)
        .margin_top(6).margin_bottom(6).margin_start(12).margin_end(12).build();
    top.append(&search);
    top.append(&tabs);
    let bar = adw::Clamp::builder().maximum_size(600).child(&top).build();

    let column = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(24)
        .margin_top(12).margin_bottom(24).margin_start(12).margin_end(12).build();
    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&adw::Clamp::builder().maximum_size(600).child(&column).build())
        .build();

    // My Themes stays, empty, for its import button
    let mine_title = tr("My Themes");
    let lists: Vec<(String, Vec<Entry>)> = sections_list().into_iter()
        .filter(|(t, e)| !e.is_empty() || *t == mine_title).collect();
    let mine = lists.iter().position(|(t, _)| *t == mine_title);
    let mut sections: Vec<Section> = Vec::new();
    for (title, _) in &lists {
        let group = adw::PreferencesGroup::builder().title(title).build();
        let button = gtk::ToggleButton::builder().label(title).css_classes(["flat"]).build();
        if let Some(s) = sections.first() {
            button.set_group(Some(&s.button));
        }
        tabs.append(&button);
        column.append(&group);
        sections.push(Section { group, button });
    }
    let page = Rc::new(Page {
        scrolled, column, search, sections, mine,
        rows: RefCell::new(Vec::new()),
        select,
    });

    for (i, (_, entries)) in lists.into_iter().enumerate() {
        for e in entries {
            page.add(i, e, current);
        }
    }
    page.update_groups();
    page.open_on_current();

    if let Some(i) = page.mine {
        let import = gtk::Button::builder().label(tr("Import…")).valign(gtk::Align::Center)
            .css_classes(["flat"]).tooltip_text(tr("Add a theme file of Black Box, Ghostty, Ptyxis or Tilix")).build();
        let p = Rc::downgrade(&page);
        import.connect_clicked(move |_| {
            if let Some(p) = p.upgrade() {
                guard("import theme", (), || p.import());
            }
        });
        page.sections[i].group.set_header_suffix(Some(&import));
    }
    for (i, s) in page.sections.iter().enumerate() {
        let p = Rc::downgrade(&page);
        s.button.connect_clicked(move |_| {
            if let Some(p) = p.upgrade() {
                guard("theme section", (), || p.go_to_section(i));
            }
        });
    }
    let p = Rc::downgrade(&page);
    page.scrolled.vadjustment().connect_value_changed(move |_| {
        if let Some(p) = p.upgrade() {
            guard("theme section", (), || p.update_section());
        }
    });
    let p = Rc::downgrade(&page);
    page.search.connect_search_changed(move |_| {
        if let Some(p) = p.upgrade() {
            guard("theme search", (), || {
                p.filter();
                p.update_section();
            });
        }
    });
    // Escape with an empty search goes back to the current theme, then closes as usual
    let p = Rc::downgrade(&page);
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed(move |_, key, _, _| guard("theme keys", glib::Propagation::Proceed, || {
        let Some(p) = p.upgrade() else { return glib::Propagation::Proceed };
        if key != gtk::gdk::Key::Escape || !p.search.text().is_empty() {
            return glib::Propagation::Proceed;
        }
        if p.current_row().is_some_and(|r| r.has_focus()) {
            return glib::Propagation::Proceed;
        }
        p.go_to_current();
        glib::Propagation::Stop
    }));

    let locate = gtk::Button::from_icon_name("find-location-symbolic");
    locate.set_tooltip_text(Some(&tr("Go to the Current Theme")));
    let p = Rc::downgrade(&page);
    locate.connect_clicked(move |_| {
        if let Some(p) = p.upgrade() {
            guard("current theme", (), || p.go_to_current());
        }
    });
    let header = adw::HeaderBar::new();
    header.pack_end(&locate);

    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.add_top_bar(&bar);
    view.set_content(Some(&page.scrolled));
    view.add_controller(keys);
    page.search.set_key_capture_widget(Some(&view));
    let nav = adw::NavigationPage::new(&view, &tr("Theme"));
    // the navigation view gives the focus to the page once it is shown: back to the theme
    let p = Rc::downgrade(&page);
    nav.connect_shown(move |_| {
        if let Some(row) = p.upgrade().and_then(|p| p.current_row()) {
            row.grab_focus();
        }
    });
    // the page state lives as long as the page
    unsafe { nav.set_data("shellman-themes", page) };
    nav
}

fn color_button(c: &gtk::gdk::RGBA, tooltip: Option<&str>, on_set: impl Fn(gtk::gdk::RGBA) + 'static) -> gtk::ColorDialogButton {
    let b = gtk::ColorDialogButton::new(Some(gtk::ColorDialog::builder().with_alpha(false).build()));
    b.set_rgba(c);
    b.set_valign(gtk::Align::Center);
    b.set_tooltip_text(tooltip);
    b.connect_rgba_notify(move |b| guard("theme color", (), || on_set(b.rgba())));
    b
}

fn base_name(id: &str) -> String {
    match id {
        FOLLOW_TERMINAL | NAUTILUS => tr("My Theme"),
        _ => colors::theme_name(id)
            .or_else(|| colors::builtin().into_iter().find(|(i, _)| *i == id).map(|(_, n)| n))
            .unwrap_or_else(|| tr("My Theme")),
    }
}

fn free_name(base: &str) -> String {
    (1..).map(|n| if n == 1 { base.to_string() } else { format!("{base} {n}") })
        .find(|n| !colors::mine_taken(n, None))
        .unwrap_or_default()
}

/// Editor of a theme of the user, starting from the colors of theme `base`
fn editor(base: &str, done: Rc<dyn Fn(&str)>, dialog: glib::WeakRef<adw::PreferencesDialog>) -> adw::NavigationPage {
    let editing = match colors::split_id(base) {
        Some((Source::Mine, file)) => Some(file.to_string()),
        _ => None,
    };
    let v = Rc::new(RefCell::new(colors::editable(base)));
    let content = adw::PreferencesPage::new();

    let about = adw::PreferencesGroup::new();
    let name = adw::EntryRow::builder().title(tr("Name")).build();
    name.set_text(&match &editing {
        Some(_) => colors::theme_name(base).unwrap_or_default(),
        None => free_name(&tr("{theme} (Custom)").replace("{theme}", &base_name(base))),
    });
    let (preview, colors) = swatch(v.borrow().clone());
    let look = adw::ActionRow::builder().title(tr("Preview")).build();
    look.add_suffix(&preview);
    about.add(&name);
    about.add(&look);
    content.add(&about);

    let refresh = {
        let (v, colors, preview) = (v.clone(), colors.clone(), preview.clone());
        Rc::new(move || {
            *colors.borrow_mut() = v.borrow().clone();
            preview.queue_draw();
        })
    };
    let main = adw::PreferencesGroup::builder().title(tr("Text and Background")).build();
    type Field = fn(&mut Variant) -> &mut Option<gtk::gdk::RGBA>;
    let fields: [(String, Field); 3] = [
        (tr("Text"), |v| &mut v.fg),
        (tr("Background"), |v| &mut v.bg),
        (tr("Cursor"), |v| &mut v.cursor),
    ];
    for (title, field) in fields {
        let current = field(&mut v.borrow_mut()).unwrap_or(gtk::gdk::RGBA::BLACK);
        let (v2, r) = (v.clone(), refresh.clone());
        let button = color_button(&current, None, move |c| {
            *field(&mut v2.borrow_mut()) = Some(c);
            r();
        });
        let row = adw::ActionRow::builder().title(&title).build();
        row.add_suffix(&button);
        row.set_activatable_widget(Some(&button));
        main.add(&row);
    }
    content.add(&main);

    let palette = adw::PreferencesGroup::builder()
        .title(tr("Palette"))
        .description(tr("The 16 colors used by programs such as ls or git"))
        .build();
    let names = [tr("Black"), tr("Red"), tr("Green"), tr("Yellow"), tr("Blue"), tr("Magenta"), tr("Cyan"), tr("White")];
    for (title, offset) in [(tr("Normal"), 0), (tr("Bright"), 8)] {
        let row = adw::ActionRow::builder().title(&title).build();
        let buttons = gtk::Box::builder().spacing(4).valign(gtk::Align::Center).build();
        for (i, color_name) in names.iter().enumerate() {
            let index = offset + i;
            let current = v.borrow().palette.get(index).copied().unwrap_or(gtk::gdk::RGBA::BLACK);
            let (v2, r) = (v.clone(), refresh.clone());
            buttons.append(&color_button(&current, Some(color_name), move |c| {
                if let Some(slot) = v2.borrow_mut().palette.get_mut(index) {
                    *slot = c;
                }
                r();
            }));
        }
        row.add_suffix(&buttons);
        palette.add(&row);
    }
    content.add(&palette);

    let save = gtk::Button::builder().label(tr("Save")).css_classes(["suggested-action"]).build();
    name.connect_changed(|e| {
        e.remove_css_class("error");
        e.set_tooltip_text(None);
    });
    let (v2, entry) = (v.clone(), name.clone());
    save.connect_clicked(move |_| guard("save theme", (), || {
        let text = entry.text().trim().to_string();
        let file = colors::theme_file(&text);
        let error = |e: &adw::EntryRow, msg: String| {
            e.add_css_class("error");
            e.set_tooltip_text(Some(&msg));
            e.grab_focus();
            if let Some(d) = dialog.upgrade() {
                d.add_toast(adw::Toast::new(&msg));
            }
        };
        if file.is_empty() {
            return error(&entry, tr("Give the theme a name"));
        }
        if colors::mine_taken(&text, editing.as_deref()) {
            return error(&entry, tr("A theme with this name already exists"));
        }
        let Some(id) = colors::save_mine(&text, &v2.borrow(), editing.as_deref()) else {
            return error(&entry, tr("The theme could not be saved"));
        };
        done(&id);
        if let Some(d) = dialog.upgrade() {
            d.pop_subpage();
        }
    }));

    let header = adw::HeaderBar::new();
    header.pack_end(&save);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&content));
    let title = if editing_title(base) { tr("Edit Theme") } else { tr("New Theme") };
    adw::NavigationPage::new(&view, &title)
}

fn editing_title(base: &str) -> bool {
    matches!(colors::split_id(base), Some((Source::Mine, _)))
}

/// The Theme row, and the row that makes a theme of the user from the current one
pub fn add_rows(group: &adw::PreferencesGroup, dialog: &adw::PreferencesDialog) {
    let current = settings::with(|s| s.palette.clone());
    let row = adw::ActionRow::builder()
        .title(tr("Theme"))
        .subtitle(name_of(&current))
        .use_markup(false)
        .activatable(true)
        .build();
    let (preview, colors) = swatch(colors::preview(&current));
    row.add_suffix(&preview);
    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    let custom = adw::ActionRow::builder().activatable(true).build();
    custom.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    let label_custom = {
        let custom = custom.clone();
        move |id: &str| if editing_title(id) {
            custom.set_title(&tr("Edit This Theme"));
            custom.set_subtitle(&tr("Change its colors or its name"));
        } else {
            custom.set_title(&tr("Customize This Theme"));
            custom.set_subtitle(&tr("Make your own theme from its colors"));
        }
    };
    label_custom(&current);
    let r = row.clone();
    let select: Rc<dyn Fn(&str)> = Rc::new(move |id: &str| {
        let id = id.to_string();
        settings::update(|s| s.palette = id.clone());
        r.set_subtitle(&name_of(&id));
        *colors.borrow_mut() = colors::preview(&id);
        preview.queue_draw();
        label_custom(&id);
    });
    let (d, sel) = (dialog.downgrade(), select.clone());
    row.connect_activated(move |_| guard("theme page", (), || {
        let Some(d) = d.upgrade() else { return };
        let current = settings::with(|s| s.palette.clone());
        d.push_subpage(&page(&current, sel.clone()));
    }));
    let d = dialog.downgrade();
    custom.connect_activated(move |_| guard("theme editor", (), || {
        let Some(dlg) = d.upgrade() else { return };
        let current = settings::with(|s| s.palette.clone());
        dlg.push_subpage(&editor(&current, select.clone(), d.clone()));
    }));
    group.add(&row);
    group.add(&custom);
}

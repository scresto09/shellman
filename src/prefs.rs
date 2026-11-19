use adw::prelude::*;
use gtk::{gdk, glib, pango};

use crate::backdrop;
use crate::config;
use crate::i18n::tr;
use crate::log;
use crate::util::guard;
use crate::pane::system_monospace_font;
use crate::settings;

thread_local! {
    static CAPTURING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

thread_local! {
    static LABELS: std::cell::RefCell<Vec<(String, glib::WeakRef<gtk::Label>)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

fn refresh_labels() {
    LABELS.with(|l| for (action, label) in l.borrow().iter() {
        if let Some(label) = label.upgrade() {
            label.set_text(&accel_text(&settings::accels(action)));
        }
    });
}

pub fn capturing() -> bool {
    CAPTURING.get()
}

pub(crate) fn accel_text(accels: &[String]) -> String {
    let labels: Vec<String> = accels.iter()
        .filter_map(gtk::accelerator_parse)
        .map(|(k, m)| gtk::accelerator_get_label(k, m).to_string())
        .collect();
    if labels.is_empty() { tr("Disabled") } else { labels.join(&tr(" or ")) }
}

fn acceptable(key: gdk::Key, mods: gdk::ModifierType) -> bool {
    use gtk::glib::translate::IntoGlib;
    let f_key = (gdk::Key::F1.into_glib()..=gdk::Key::F35.into_glib()).contains(&key.into_glib());
    f_key || mods.intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK
                             | gdk::ModifierType::SUPER_MASK)
}

fn capture_shortcut(win: &gtk::Window, parent: &impl IsA<gtk::Widget>, label: &str,
                    done: impl Fn(Option<String>) + 'static) {
    let dialog = adw::AlertDialog::new(
        Some(&tr("New Shortcut")),
        Some(&tr("Press the key combination for “{action}”.\n\nEscape: cancel · Backspace: disable").replace("{action}", label)));
    dialog.add_response("cancel", &tr("Cancel"));
    dialog.set_close_response("cancel");
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let d = dialog.clone();
    let done = std::rc::Rc::new(done);
    keys.connect_key_pressed(move |ctrl, key, code, state| guard("shortcut capture", glib::Propagation::Stop, || {
        if ctrl.current_event().and_then(|e| e.downcast::<gdk::KeyEvent>().ok()).is_some_and(|e| e.is_modifier()) {
            return glib::Propagation::Proceed;
        }
        let mods = state & gtk::accelerator_get_default_mod_mask();
        let base = crate::util::base_key(ctrl, key, code);
        if mods.is_empty() && base == gdk::Key::Escape {
            d.close();
        } else if mods.is_empty() && base == gdk::Key::BackSpace {
            done(None);
            d.close();
        } else if acceptable(base, mods) {
            done(Some(gtk::accelerator_name(base, mods).to_string()));
            d.close();
        } else {
            d.set_body(&tr("Use Ctrl, Alt or Super with the key, or a function key (F1…F12).\n\nEscape: cancel · Backspace: disable"));
            if let Some(w) = ctrl.widget() { w.error_bell(); }
        }
        glib::Propagation::Stop
    }));
    dialog.add_controller(keys);
    CAPTURING.set(true);
    crate::panel::mute_window_shortcuts(win, true);
    let w = win.clone();
    dialog.connect_closed(move |_| {
        CAPTURING.set(false);
        guard("shortcut capture", (), || crate::panel::mute_window_shortcuts(&w, false));
    });
    dialog.present(Some(parent));
}

fn image_name(path: &str) -> String {
    if path.is_empty() {
        return tr("None");
    }
    std::path::Path::new(path).file_name()
        .map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned())
}

// 3 x 3 radio buttons laid out like the terminal area
fn position_grid(current: &str) -> gtk::Grid {
    let grid = gtk::Grid::builder()
        .valign(gtk::Align::Center)
        .margin_top(4)
        .margin_bottom(4)
        .build();
    let mut first: Option<gtk::CheckButton> = None;
    for (i, (id, label, _, _)) in backdrop::POSITIONS.iter().enumerate() {
        let b = gtk::CheckButton::builder()
            .tooltip_text(tr(label))
            .active(*id == current)
            .build();
        b.update_property(&[gtk::accessible::Property::Label(&tr(label))]);
        if let Some(f) = &first {
            b.set_group(Some(f));
        } else {
            first = Some(b.clone());
        }
        b.connect_toggled(move |b| {
            if b.is_active() {
                settings::update(|s| s.bg_position = id.to_string());
            }
        });
        grid.attach(&b, (i % 3) as i32, (i / 3) as i32, 1, 1);
    }
    grid
}

fn background_groups(s: &settings::Settings) -> [adw::PreferencesGroup; 2] {
    let base = adw::PreferencesGroup::builder().title(tr("Background")).build();
    let see_through = adw::SwitchRow::builder()
        .title(tr("Terminal Transparency"))
        .subtitle(if crate::seethrough::available() {
            tr("The desktop and the windows behind Nautilus stay visible. To blur them, install the Blur my Shell extension.")
        } else {
            tr("Unavailable: the display has no compositor")
        })
        .active(s.see_through)
        .sensitive(crate::seethrough::available())
        .build();

    let group = adw::PreferencesGroup::builder().title(tr("Background Image")).build();
    let row = adw::ActionRow::builder().title(tr("Image")).subtitle(image_name(&s.bg_image)).build();
    let choose = gtk::Button::from_icon_name("document-open-symbolic");
    choose.set_tooltip_text(Some(&tr("Choose an Image…")));
    let clear = gtk::Button::from_icon_name("edit-clear-symbolic");
    clear.set_tooltip_text(Some(&tr("Remove the Image")));
    for b in [&choose, &clear] {
        b.add_css_class("flat");
        b.set_valign(gtk::Align::Center);
        row.add_suffix(b);
    }

    let combo = |title: &str, labels: Vec<String>, selected: usize| {
        let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
        adw::ComboRow::builder()
            .title(title)
            .model(&gtk::StringList::new(&labels))
            .selected(selected as u32)
            .build()
    };
    let fit = combo(&tr("Layout"), backdrop::FITS.iter().map(|f| tr(f.1)).collect(),
                    backdrop::FITS.iter().position(|f| f.0 == s.bg_fit).unwrap_or(3));
    let position = adw::ActionRow::builder().title(tr("Position")).build();
    position.add_suffix(&position_grid(&s.bg_position));

    let shared = adw::SwitchRow::builder()
        .title(tr("One Image per Tab"))
        .subtitle(tr("When off, each split terminal shows the whole image"))
        .active(s.bg_shared)
        .build();
    shared.connect_active_notify(|r| {
        let on = r.is_active();
        settings::update(|s| s.bg_shared = on);
    });

    let opacity = adw::SpinRow::with_range(0.0, 100.0, 5.0);
    opacity.set_title(&tr("Background Opacity"));
    opacity.set_subtitle(&tr("In percent; the lower it is, the more the image or the desktop shows through"));
    opacity.set_value(s.bg_opacity as f64);
    opacity.connect_value_notify(|r| {
        let v = r.value() as i32;
        settings::update(|s| s.bg_opacity = v);
    });
    let blur = adw::SpinRow::with_range(0.0, 100.0, 1.0);
    blur.set_title(&tr("Blur"));
    blur.set_subtitle(&tr("In pixels"));
    blur.set_value(s.bg_blur as f64);
    let pending: std::rc::Rc<std::cell::RefCell<Option<glib::SourceId>>> = Default::default();
    blur.connect_value_notify(move |r| {
        let v = r.value() as i32;
        if let Some(id) = pending.take() {
            id.remove();
        }
        let p = pending.clone();
        *pending.borrow_mut() = Some(glib::timeout_add_local_once(std::time::Duration::from_millis(200), move || {
            p.take();
            settings::update(|s| s.bg_blur = v);
        }));
    });

    let refresh = {
        let (row, clear, opacity, blur) = (row.clone(), clear.clone(), opacity.clone(), blur.clone());
        let (fit, position, shared) = (fit.clone(), position.clone(), shared.clone());
        move || guard("background rows", (), || {
            let (path, f, through) = settings::with(|s| (s.bg_image.clone(), s.bg_fit.clone(), s.see_through));
            row.set_subtitle(&image_name(&path));
            for w in [clear.upcast_ref::<gtk::Widget>(), fit.upcast_ref(), shared.upcast_ref(), blur.upcast_ref()] {
                w.set_sensitive(!path.is_empty());
            }
            opacity.set_sensitive(!path.is_empty() || (through && crate::seethrough::available()));
            position.set_sensitive(!path.is_empty() && backdrop::uses_position(&f));
        })
    };
    refresh();

    let r = refresh.clone();
    see_through.connect_active_notify(move |w| {
        let on = w.is_active();
        settings::update(|s| s.see_through = on);
        r();
    });

    let r = refresh.clone();
    fit.connect_selected_notify(move |c| {
        if let Some(f) = backdrop::FITS.get(c.selected() as usize) {
            settings::update(|s| s.bg_fit = f.0.to_string());
            r();
        }
    });

    let r = refresh.clone();
    clear.connect_clicked(move |_| guard("background image", (), || {
        settings::update(|s| s.bg_image.clear());
        r();
    }));
    choose.connect_clicked(move |b| guard("background image", (), || {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some(&tr("Images")));
        filter.add_mime_type("image/*");
        let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dialog = gtk::FileDialog::builder()
            .title(tr("Background Image"))
            .modal(true)
            .filters(&filters)
            .default_filter(&filter)
            .build();
        let current = settings::with(|s| s.bg_image.clone());
        if !current.is_empty() {
            dialog.set_initial_file(Some(&gtk::gio::File::for_path(&current)));
        }
        let window = b.root().and_downcast::<gtk::Window>();
        let r = refresh.clone();
        dialog.open(window.as_ref(), gtk::gio::Cancellable::NONE, move |res| guard("background image", (), || {
            let Some(path) = res.ok().and_then(|f| f.path()) else { return };
            let path = path.to_string_lossy().into_owned();
            settings::update(|s| s.bg_image = path);
            r();
        }));
    }));

    group.add(&row);
    group.add(&fit);
    group.add(&position);
    group.add(&shared);
    group.add(&blur);
    base.add(&see_through);
    base.add(&opacity);
    [base, group]
}

fn colors_group(s: &settings::Settings, dialog: &adw::PreferencesDialog) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title(tr("Colors")).build();

    let styles = ["system", "light", "dark"];
    let style = adw::ComboRow::builder()
        .title(tr("Style"))
        .model(&gtk::StringList::new(&[&tr("Follow System"), &tr("Light"), &tr("Dark")]))
        .selected(styles.iter().position(|v| *v == s.style).unwrap_or(0) as u32)
        .build();
    style.connect_selected_notify(move |r| {
        let v = styles.get(r.selected() as usize).unwrap_or(&"system").to_string();
        settings::update(|s| s.style = v);
    });
    group.add(&style);

    crate::themes::add_rows(&group, dialog);
    group
}

pub fn show_preferences(win: &gtk::Window) {
    let s = settings::get();
    let dialog = adw::PreferencesDialog::builder()
        .title(tr("Terminal Preferences"))
        .search_enabled(true)
        .build();
    let page_general = adw::PreferencesPage::builder()
        .title(tr("General"))
        .icon_name("preferences-system-symbolic")
        .build();
    let page_look = adw::PreferencesPage::builder()
        .title(tr("Appearance"))
        .icon_name("preferences-desktop-appearance-symbolic")
        .build();
    let page_keys = adw::PreferencesPage::builder()
        .title(tr("Shortcuts"))
        .icon_name("preferences-desktop-keyboard-shortcuts-symbolic")
        .build();

    let general = adw::PreferencesGroup::builder().title(tr("Behavior")).build();
    let modes = settings::Sync::ALL;
    let labels = [tr("Both Ways"), tr("The Terminal Follows Nautilus"), tr("Nautilus Follows the Terminal"), tr("Off")];
    let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
    let sync = adw::ComboRow::builder()
        .title(tr("Folder Sync"))
        .subtitle(tr("Between the terminal and the Nautilus tab; the shell changes folder when no command is running"))
        .model(&gtk::StringList::new(&labels))
        .selected(modes.iter().position(|m| *m == s.sync).unwrap_or(0) as u32)
        .factory(&label_factory(true))
        .list_factory(&label_factory(false))
        .build();
    sync.connect_selected_notify(move |r| {
        if let Some(m) = modes.get(r.selected() as usize).copied() {
            settings::update(|s| s.sync = m);
        }
    });
    general.add(&sync);

    let per_tab = adw::SwitchRow::builder()
        .title(tr("One Terminal per Tab"))
        .subtitle(tr("When off, one terminal per window follows the active tab. Applies to new windows."))
        .active(s.per_tab)
        .build();
    per_tab.connect_active_notify(|r| {
        let on = r.is_active();
        settings::update(|s| s.per_tab = on);
    });
    general.add(&per_tab);

    let new_tabs = adw::SwitchRow::builder()
        .title(tr("Show the Terminal in New Tabs"))
        .subtitle(tr("With one terminal per tab, showing or hiding the terminal only affects the current tab"))
        .active(s.show_in_new_tabs)
        .build();
    new_tabs.connect_active_notify(|r| {
        let on = r.is_active();
        settings::update(|s| s.show_in_new_tabs = on);
    });
    general.add(&new_tabs);

    let ls_links = adw::SwitchRow::builder()
        .title(tr("Clickable Names in ls"))
        .subtitle(tr("Adds --hyperlink to ls: right-click a name to open the file or go to the folder. Applies to new terminals."))
        .active(s.ls_links)
        .build();
    ls_links.connect_active_notify(|r| {
        let on = r.is_active();
        settings::update(|s| s.ls_links = on);
    });
    general.add(&ls_links);

    let notify = adw::SwitchRow::builder()
        .title(tr("Notify When a Command Finishes"))
        .subtitle(tr("When it ran for more than 10 seconds while the terminal was hidden or the window in the background"))
        .active(s.notify_done)
        .build();
    notify.connect_active_notify(|r| {
        let on = r.is_active();
        settings::update(|s| s.notify_done = on);
    });
    general.add(&notify);
    page_general.add(&general);

    let size = adw::PreferencesGroup::builder().title(tr("Size")).build();
    let rows = adw::SpinRow::with_range(1.0, 100.0, 1.0);
    rows.set_title(&tr("Height When Opened"));
    rows.set_subtitle(&tr("In lines; also the minimum height of a terminal"));
    rows.set_value(s.rows as f64);
    rows.connect_value_notify(|r| {
        let v = r.value() as i32;
        settings::update(|s| s.rows = v);
    });
    size.add(&rows);

    let scrollback = adw::SpinRow::with_range(0.0, 1_000_000.0, 1000.0);
    scrollback.set_title(&tr("Scrollback Lines"));
    scrollback.set_value(s.scrollback as f64);
    scrollback.connect_value_notify(|r| {
        let v = r.value() as i64;
        settings::update(|s| s.scrollback = v);
    });
    size.add(&scrollback);
    page_general.add(&size);

    let fonts = adw::PreferencesGroup::builder().title(tr("Font")).build();
    let system = adw::SwitchRow::builder()
        .title(tr("Use System Monospace Font"))
        .active(s.font.is_none())
        .build();
    let current = s.font.clone().or_else(system_monospace_font).unwrap_or_else(|| "Monospace 11".into());
    let button = gtk::FontDialogButton::new(Some(gtk::FontDialog::new()));
    button.set_font_desc(&pango::FontDescription::from_string(&current));
    button.set_valign(gtk::Align::Center);
    let font_row = adw::ActionRow::builder().title(tr("Font")).build();
    font_row.add_suffix(&button);
    font_row.set_sensitive(s.font.is_some());
    let b = button.clone();
    let fr = font_row.clone();
    system.connect_active_notify(move |r| guard("system font", (), || {
        let sys = r.is_active();
        fr.set_sensitive(!sys);
        let font = (!sys).then(|| b.font_desc().map(|d| d.to_str().to_string())).flatten();
        settings::update(|s| s.font = font);
    }));
    let sys = system.clone();
    button.connect_font_desc_notify(move |b| {
        if !sys.is_active() {
            let font = b.font_desc().map(|d| d.to_str().to_string());
            settings::update(|s| s.font = font);
        }
    });
    fonts.add(&system);
    fonts.add(&font_row);
    page_look.add(&fonts);

    let cursor = adw::PreferencesGroup::builder().title(tr("Cursor and Prompts")).build();
    let bar = adw::SwitchRow::builder()
        .title(tr("Bar Cursor at the Prompt"))
        .subtitle(tr("A block cursor while a command runs"))
        .active(s.cursor_bar)
        .build();
    bar.connect_active_notify(|r| {
        let on = r.is_active();
        settings::update(|s| s.cursor_bar = on);
    });
    cursor.add(&bar);
    let marks = adw::SwitchRow::builder()
        .title(tr("Prompt Marks"))
        .subtitle(tr("A mark along the scrollbar for each prompt"))
        .active(s.prompt_marks)
        .build();
    marks.connect_active_notify(|r| {
        let on = r.is_active();
        settings::update(|s| s.prompt_marks = on);
    });
    cursor.add(&marks);
    let buttons = adw::SwitchRow::builder()
        .title(tr("Prompt Buttons"))
        .subtitle(tr("Buttons near the scrollbar to go from one prompt to another"))
        .active(s.prompt_buttons)
        .build();
    buttons.connect_active_notify(|r| {
        let on = r.is_active();
        settings::update(|s| s.prompt_buttons = on);
    });
    cursor.add(&buttons);
    page_look.add(&cursor);
    page_look.add(&colors_group(&s, &dialog));
    for g in background_groups(&s) {
        page_look.add(&g);
    }

    let mut labels = Vec::new();
    let mut first_group: Option<adw::PreferencesGroup> = None;
    for (group_title, actions) in config::SHORTCUT_GROUPS {
        let keys = adw::PreferencesGroup::builder().title(tr(group_title)).build();
        if first_group.is_none() {
            keys.set_description(Some(&tr("Click an action, then press the new key combination")));
            first_group = Some(keys.clone());
        }
        for action in *actions {
            let Some((action, title)) = config::ACTIONS.iter().find(|(a, _)| a == action) else { continue };
            let title = tr(title);
            let row = adw::ActionRow::builder().title(&title).activatable(true).build();
            let label = gtk::Label::new(Some(&accel_text(&settings::accels(action))));
            label.add_css_class("dim-label");
            row.add_suffix(&label);
            let (a, t, w, dlg) = (action.to_string(), title, win.clone(), dialog.clone());
            row.connect_activated(move |_| guard("shortcut row", (), || {
                let a = a.clone();
                capture_shortcut(&w, &dlg, &t, move |accel| {
                    if let Some(other) = accel.as_deref().and_then(|k| settings::owner_of(k, &a)) {
                        log!("shortcut removed from \"{other}\" (reassigned)");
                    }
                    settings::set_accel(&a, accel);
                    refresh_labels();
                });
            }));
            keys.add(&row);
            labels.push((action.to_string(), label.downgrade()));
        }
        page_keys.add(&keys);
    }
    LABELS.with(|l| *l.borrow_mut() = labels);
    let reset = gtk::Button::with_label(&tr("Reset"));
    reset.add_css_class("flat");
    reset.set_valign(gtk::Align::Center);
    reset.set_tooltip_text(Some(&tr("Restore all default shortcuts")));
    reset.connect_clicked(|_| guard("reset shortcuts", (), || {
        settings::reset_shortcuts();
        refresh_labels();
    }));
    if let Some(g) = first_group {
        g.set_header_suffix(Some(&reset));
    }

    let fixed = adw::PreferencesGroup::builder()
        .title(tr("Built In"))
        .description(tr("Provided by the terminal, they cannot be changed"))
        .build();
    for (title, keys) in [(tr("Previous Prompt"), "<Control><Shift>Left"),
                          (tr("Next Prompt"), "<Control><Shift>Right")] {
        let row = adw::ActionRow::builder().title(title).build();
        let label = gtk::Label::new(Some(&accel_text(&[keys.to_string()])));
        label.add_css_class("dim-label");
        row.add_suffix(&label);
        fixed.add(&row);
    }
    let row = adw::ActionRow::builder()
        .title(tr("Move the Cursor at the Prompt"))
        .build();
    let label = gtk::Label::new(Some(&tr("Alt+Click")));
    label.add_css_class("dim-label");
    row.add_suffix(&label);
    fixed.add(&row);
    page_keys.add(&fixed);

    dialog.add(&page_general);
    dialog.add(&page_look);
    dialog.add(&page_keys);
    dialog.present(Some(win));
}

pub fn show_about(win: &gtk::Window) {
    let about = adw::AboutDialog::builder()
        .application_name("ShellMan")
        .application_icon("shellman")
        .version(env!("CARGO_PKG_VERSION"))
        .developer_name("Sylvain Cresto")
        .developers(["Sylvain Cresto"])
        .copyright("© 2026 Sylvain Cresto")
        .license_type(gtk::License::Gpl30)
        .website(env!("CARGO_PKG_HOMEPAGE"))
        .issue_url(concat!(env!("CARGO_PKG_REPOSITORY"), "/issues"))
        .comments(tr("An embedded terminal at the bottom of Nautilus (GNOME Files) windows. It follows the displayed folder, and the files selected in Nautilus are available in the SEL variable."))
        .build();
    about.add_credit_section(Some(&tr("Built with")), &[&tr("VTE (GNOME terminal widget)"), "gtk4-rs, libadwaita-rs"]);
    about.present(Some(win));
}

fn label_factory(wrap: bool) -> gtk::SignalListItemFactory {
    let f = gtk::SignalListItemFactory::new();
    f.connect_setup(move |_, li| guard("combo label", (), || {
        let Some(li) = li.downcast_ref::<gtk::ListItem>() else { return };
        let label = gtk::Label::builder().xalign(if wrap { 1.0 } else { 0.0 }).build();
        if wrap {
            label.set_wrap(true);
            label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            label.set_justify(gtk::Justification::Right);
            label.set_max_width_chars(14);
        }
        li.set_child(Some(&label));
    }));
    f.connect_bind(|_, li| guard("combo label", (), || {
        let Some(li) = li.downcast_ref::<gtk::ListItem>() else { return };
        let text = li.item().and_downcast::<gtk::StringObject>().map(|o| o.string()).unwrap_or_default();
        if let Some(label) = li.child().and_downcast::<gtk::Label>() {
            label.set_label(&text);
        }
    }));
    f
}

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use gtk::gdk::RGBA;
use gtk::gio::{self, prelude::*};
use gtk::glib;

use crate::settings;
use crate::util::guard;

pub const FOLLOW_TERMINAL: &str = "terminal";
pub const NAUTILUS: &str = "nautilus";

const GNOME: &str = "gnome";
const BUILTIN: &[(&str, &str)] = &[
    ("gnome", include_str!("../palettes/gnome.palette")),
    ("gnome-high-contrast", include_str!("../palettes/gnome-high-contrast.palette")),
    ("Ubuntu", include_str!("../palettes/Ubuntu.palette")),
    ("tango", include_str!("../palettes/tango.palette")),
    ("linux", include_str!("../palettes/linux.palette")),
    ("xterm", include_str!("../palettes/xterm.palette")),
    ("campbell", include_str!("../palettes/campbell.palette")),
    ("Vs Code", include_str!("../palettes/Vs Code.palette")),
    ("solarized", include_str!("../palettes/solarized.palette")),
    ("dracula", include_str!("../palettes/dracula.palette")),
    ("nord", include_str!("../palettes/nord.palette")),
    ("Horizon", include_str!("../palettes/Horizon.palette")),
];

/// fg None: text color of the theme; bg None: transparent, the file view shows through
#[derive(Clone, Default)]
pub struct Variant {
    pub fg: Option<RGBA>,
    pub bg: Option<RGBA>,
    pub cursor: Option<RGBA>,
    pub palette: Vec<RGBA>,
}

#[derive(Clone)]
struct Scheme {
    light: Variant,
    dark: Variant,
    force_dark: Option<bool>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Terminal {
    Ptyxis,
    GnomeTerminal,
}

impl Terminal {
    pub fn name(self) -> &'static str {
        match self {
            Terminal::Ptyxis => "Ptyxis",
            Terminal::GnomeTerminal => "GNOME Terminal",
        }
    }
}

// Ptyxis palettes: colors in [Light] and [Dark], or in [Palette] for both styles
fn parse(text: &str) -> Option<(String, Scheme)> {
    let kf = glib::KeyFile::new();
    kf.load_from_data(text, glib::KeyFileFlags::NONE).ok()?;
    let variant = |group: &str| -> Option<Variant> {
        let get = |key: &str| kf.string(group, key).ok().and_then(|s| RGBA::parse(s.as_str()).ok());
        let palette: Vec<RGBA> = (0..16).filter_map(|i| get(&format!("Color{i}"))).collect();
        Some(Variant {
            fg: Some(get("Foreground")?),
            bg: Some(get("Background")?),
            cursor: get("Cursor"),
            palette: if palette.len() == 16 { palette } else { Vec::new() },
        })
    };
    let name = kf.string("Palette", "Name").map(|s| s.trim().chars().take(100).collect()).unwrap_or_default();
    let scheme = match (variant("Light"), variant("Dark")) {
        (Some(light), Some(dark)) => Scheme { light, dark, force_dark: None },
        _ => single(variant("Palette")?),
    };
    Some((name, scheme))
}

/// (id, name) of the bundled palettes, sorted by name
pub fn builtin() -> Vec<(&'static str, String)> {
    let mut list: Vec<_> = BUILTIN.iter()
        .filter_map(|(id, text)| parse(text).map(|(name, ..)| (*id, if name.is_empty() { id.to_string() } else { name })))
        .collect();
    list.sort_by_key(|(_, name)| name.to_lowercase());
    list
}

fn builtin_scheme(id: &str) -> Option<Scheme> {
    let (_, text) = BUILTIN.iter().find(|(i, _)| i.eq_ignore_ascii_case(id))?;
    parse(text).map(|(_, scheme)| scheme)
}

fn gnome() -> Scheme {
    builtin_scheme(GNOME).unwrap_or(Scheme { light: Variant::default(), dark: Variant::default(), force_dark: None })
}

fn schema(id: &str) -> Option<gio::SettingsSchema> {
    gio::SettingsSchemaSource::default()?.lookup(id, true)
}

fn open(id: &str, path: Option<&str>) -> Option<(gio::Settings, gio::SettingsSchema)> {
    let schema = schema(id)?;
    if schema.path().is_none() && path.is_none() {
        return None;
    }
    let s = gio::Settings::new_full(&schema, None::<&gio::SettingsBackend>, path);
    Some((s, schema))
}

fn string(s: &(gio::Settings, gio::SettingsSchema), key: &str) -> Option<String> {
    s.1.has_key(key).then(|| s.0.string(key).to_string())
}

fn boolean(s: &(gio::Settings, gio::SettingsSchema), key: &str) -> Option<bool> {
    s.1.has_key(key).then(|| s.0.boolean(key))
}

fn style(value: Option<String>) -> Option<bool> {
    match value.as_deref() {
        Some("dark") => Some(true),
        Some("light") => Some(false),
        _ => None,
    }
}

fn installed(t: Terminal) -> bool {
    match t {
        Terminal::Ptyxis => schema("org.gnome.Ptyxis").is_some(),
        Terminal::GnomeTerminal => schema("org.gnome.Terminal.Legacy.Profile").is_some(),
    }
}

/// The user's terminal, as chosen by xdg-terminal-exec
pub fn default_terminal() -> Option<Terminal> {
    let desktops: Vec<String> = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default()
        .split(':').filter(|d| !d.is_empty()).map(str::to_lowercase).collect();
    let mut dirs: Vec<PathBuf> = vec![glib::user_config_dir()];
    dirs.extend(glib::system_config_dirs());
    dirs.push(glib::user_data_dir().join("xdg-terminal-exec"));
    dirs.extend(glib::system_data_dirs().into_iter().map(|d| d.join("xdg-terminal-exec")));
    let names: Vec<String> = desktops.iter().map(|d| format!("{d}-xdg-terminals.list"))
        .chain(std::iter::once("xdg-terminals.list".into())).collect();
    for dir in &dirs {
        for name in &names {
            if let Some(t) = first_entry(&dir.join(name)) {
                return t;
            }
        }
    }
    [Terminal::Ptyxis, Terminal::GnomeTerminal].into_iter().find(|t| installed(*t))
}

const THEME_MAX: u64 = 256 * 1024;

// Files from other programs or the user: only regular files of a reasonable size are read,
// so that a FIFO, a device or a huge file in a theme folder cannot block or exhaust memory.
fn read_file(path: &Path, max: u64) -> Option<String> {
    use std::io::Read;
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > max {
        return None;
    }
    let mut text = String::new();
    std::fs::File::open(path).ok()?.take(max + 1).read_to_string(&mut text).ok()?;
    (text.len() as u64 <= max).then_some(text)
}

/// Some(None): another terminal comes first
fn first_entry(file: &Path) -> Option<Option<Terminal>> {
    let text = read_file(file, THEME_MAX)?;
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') || line.starts_with('-') {
            continue;
        }
        let id = line.trim_start_matches('+').split(':').next().unwrap_or("");
        let t = match id {
            "org.gnome.Ptyxis.desktop" => Terminal::Ptyxis,
            "org.gnome.Terminal.desktop" => Terminal::GnomeTerminal,
            _ => {
                if gio::DesktopAppInfo::new(id).is_some() {
                    return Some(None);
                }
                continue;
            }
        };
        if installed(t) {
            return Some(Some(t));
        }
    }
    None
}

fn ptyxis_palette(id: &str) -> Scheme {
    let file = glib::user_data_dir().join("org.gnome.Ptyxis").join("palettes").join(format!("{id}.palette"));
    if let Some((_, scheme)) = read_file(&file, THEME_MAX).as_deref().and_then(parse) {
        return scheme;
    }
    builtin_scheme(id)
        .or_else(|| BUILTIN.iter().find(|(_, t)| parse(t).is_some_and(|(n, ..)| n.eq_ignore_ascii_case(id)))
             .and_then(|(i, _)| builtin_scheme(i)))
        .unwrap_or_else(gnome)
}

fn ptyxis(watch: &mut Vec<gio::Settings>) -> Option<Scheme> {
    let app = open("org.gnome.Ptyxis", None)?;
    let uuid = string(&app, "default-profile-uuid").unwrap_or_default();
    let palette = if uuid.is_empty() {
        schema("org.gnome.Ptyxis.Profile")?.key("palette").default_value().str().map(String::from)
    } else {
        let profile = open("org.gnome.Ptyxis.Profile", Some(&format!("/org/gnome/Ptyxis/Profiles/{uuid}/")))?;
        let p = string(&profile, "palette");
        watch.push(profile.0);
        p
    };
    let mut scheme = ptyxis_palette(palette.as_deref().unwrap_or(GNOME));
    scheme.force_dark = style(string(&app, "interface-style"));
    watch.push(app.0);
    Some(scheme)
}

fn css_block<'a>(css: &'a str, selector: &str) -> Option<&'a str> {
    let start = css.find(&format!("{selector} {{"))? + selector.len() + 2;
    let len = css[start..].find('}')?;
    Some(&css[start..start + len])
}

fn css_color(block: &str, property: &str) -> Option<RGBA> {
    block.split(';').find_map(|decl| {
        let (k, v) = decl.split_once(':')?;
        (k.trim() == property).then(|| RGBA::parse(v.trim()).ok()).flatten()
    })
}

/// Text and background that a GTK 3 theme (Yaru…) gives to GNOME Terminal
fn gtk3_terminal_colors(theme: &str, dark: bool) -> Option<(RGBA, RGBA)> {
    if theme.is_empty() || theme.contains('/') || theme.starts_with('.') {
        return None;
    }
    let mut dirs = vec![glib::user_data_dir().join("themes"), glib::home_dir().join(".themes")];
    dirs.extend(glib::system_data_dirs().into_iter().map(|d| d.join("themes")));
    let dir = dirs.into_iter().map(|d| d.join(theme).join("gtk-3.0")).find(|d| d.is_dir())?;
    let file = if dark && dir.join("gtk-dark.css").is_file() { "gtk-dark.css" } else { "gtk.css" };
    let mut css = read_file(&dir.join(file), 4 * 1024 * 1024)?;
    let resource = gio::Resource::load(dir.join("gtk.gresource")).ok();
    for line in css.clone().lines() {
        let Some(path) = line.trim().strip_prefix("@import url(\"resource://")
            .and_then(|r| r.strip_suffix("\");")) else { continue };
        if let Some(data) = resource.as_ref()
            .and_then(|r| r.lookup_data(path, gio::ResourceLookupFlags::NONE).ok()) {
            css.push_str(&String::from_utf8_lossy(&data));
        }
    }
    let block = css_block(&css, "terminal-window .terminal-screen")?;
    Some((css_color(block, "color")?, css_color(block, "background-color")?))
}

fn gnome_terminal(watch: &mut Vec<gio::Settings>) -> Option<Scheme> {
    let list = open("org.gnome.Terminal.ProfilesList", None)?;
    let uuid = string(&list, "default").filter(|u| !u.is_empty())?;
    let profile = open("org.gnome.Terminal.Legacy.Profile",
                       Some(&format!("/org/gnome/terminal/legacy/profiles:/:{uuid}/")))?;
    let color = |key: &str| string(&profile, key).and_then(|s| RGBA::parse(s.as_str()).ok());
    let palette: Vec<RGBA> = profile.1.has_key("palette").then(|| profile.0.strv("palette"))
        .map(|v| v.iter().filter_map(|c| RGBA::parse(c.as_str()).ok()).collect())
        .unwrap_or_default();
    let palette = if palette.len() >= 16 { palette[..16].to_vec() } else { Vec::new() };
    let theme = boolean(&profile, "use-theme-colors").unwrap_or(true);
    let cursor = boolean(&profile, "cursor-colors-set").unwrap_or(false)
        .then(|| color("cursor-background-color")).flatten();
    let interface = open("org.gnome.desktop.interface", None);
    let gtk_theme = interface.as_ref().and_then(|i| string(i, "gtk-theme")).unwrap_or_default();
    if let Some(i) = interface {
        watch.push(i.0);
    }
    let v = |fallback: &Variant, dark: bool| {
        let (fg, bg) = if theme {
            gtk3_terminal_colors(&gtk_theme, dark).map_or((None, None), |(f, b)| (Some(f), Some(b)))
        } else {
            (color("foreground-color").or(fallback.fg), color("background-color").or(fallback.bg))
        };
        Variant {
            fg, bg, cursor,
            palette: if palette.is_empty() { fallback.palette.clone() } else { palette.clone() },
        }
    };
    let g = gnome();
    let force_dark = open("org.gnome.Terminal.Legacy.Settings", None)
        .and_then(|s| { let v = style(string(&s, "theme-variant")); watch.push(s.0); v });
    let scheme = Scheme { light: v(&g.light, false), dark: v(&g.dark, true), force_dark };
    watch.push(list.0);
    watch.push(profile.0);
    Some(scheme)
}

/// Theme files read from other terminals, and the user's own themes
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Source {
    BlackBox,
    Ghostty,
    Ptyxis,
    Tilix,
    Mine,
}

#[derive(Clone, Copy, PartialEq)]
enum Format {
    Tilix,
    Ghostty,
    Ptyxis,
}

impl Source {
    pub const ALL: [Source; 5] = [Source::BlackBox, Source::Ghostty, Source::Ptyxis, Source::Tilix, Source::Mine];

    /// palette ids of these themes: this prefix, then the file name
    pub fn prefix(self) -> &'static str {
        match self {
            Source::BlackBox => "blackbox:",
            Source::Tilix => "tilix:",
            Source::Ghostty => "ghostty:",
            Source::Ptyxis => "ptyxis:",
            Source::Mine => "mine:",
        }
    }

    pub fn terminal(self) -> Option<&'static str> {
        match self {
            Source::BlackBox => Some("Black Box"),
            Source::Tilix => Some("Tilix"),
            Source::Ghostty => Some("Ghostty"),
            Source::Ptyxis => Some("Ptyxis"),
            Source::Mine => None,
        }
    }

    // Tilix and Black Box share the JSON format of Tilix, also used for the user's themes;
    // other files there are read as Ghostty themes
    fn format(self, file: &str) -> Format {
        match self {
            Source::BlackBox | Source::Tilix => Format::Tilix,
            Source::Ptyxis => Format::Ptyxis,
            Source::Ghostty => Format::Ghostty,
            Source::Mine if file.ends_with(".json") => Format::Tilix,
            Source::Mine => Format::Ghostty,
        }
    }

    fn accepts(self, file: &str) -> bool {
        match self {
            Source::BlackBox | Source::Tilix => file.ends_with(".json"),
            Source::Ptyxis => file.ends_with(".palette"),
            _ => true,
        }
    }

    /// first directory first: a user theme hides a system one of the same file name
    fn dirs(self) -> Vec<PathBuf> {
        let system = |sub: &str| glib::system_data_dirs().into_iter().map(|d| d.join(sub)).collect::<Vec<_>>();
        let mut dirs = match self {
            Source::BlackBox => vec![
                glib::user_data_dir().join("blackbox/schemes"),
                glib::home_dir().join(".var/app/com.raggesilver.BlackBox/data/blackbox/schemes"),
            ],
            Source::Tilix => vec![glib::user_config_dir().join("tilix/schemes")],
            Source::Ghostty => vec![glib::user_config_dir().join("ghostty/themes"),
                                    glib::user_data_dir().join("ghostty/themes")],
            Source::Ptyxis => return vec![
                glib::user_data_dir().join("org.gnome.Ptyxis/palettes"),
                glib::home_dir().join(".var/app/app.devsuite.Ptyxis/data/org.gnome.Ptyxis/palettes"),
            ],
            Source::Mine => return vec![mine_dir()],
        };
        dirs.extend(system(match self {
            Source::BlackBox => "blackbox/schemes",
            Source::Tilix => "tilix/schemes",
            _ => "ghostty/themes",
        }));
        dirs
    }
}

pub fn mine_dir() -> PathBuf {
    glib::user_config_dir().join("shellman").join("themes")
}

pub fn split_id(id: &str) -> Option<(Source, &str)> {
    Source::ALL.iter().find_map(|s| id.strip_prefix(s.prefix()).map(|f| (*s, f)))
        .filter(|(_, f)| valid_file(f))
}

fn valid_file(f: &str) -> bool {
    !f.is_empty() && !f.contains('/') && !f.starts_with('.')
}

fn read(source: Source, file: &str) -> Option<String> {
    if !valid_file(file) {
        return None;
    }
    source.dirs().iter().find_map(|d| read_file(&d.join(file), THEME_MAX))
}

pub struct ThemeFile {
    pub id: String,
    pub name: String,
}

/// the themes of a source, sorted by name
pub fn themes(source: Source) -> Vec<ThemeFile> {
    let mut files: Vec<String> = source.dirs().iter()
        .filter_map(|d| std::fs::read_dir(d).ok())
        .flat_map(|e| e.flatten())
        .filter(|e| std::fs::metadata(e.path()).is_ok_and(|m| m.is_file() && m.len() <= THEME_MAX))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| valid_file(n) && source.accepts(n))
        .collect();
    files.sort();
    files.dedup();
    let mut list: Vec<ThemeFile> = files.into_iter().filter_map(|f| {
        let name = file_name_of(source, &f)?;
        Some(ThemeFile { id: format!("{}{f}", source.prefix()), name })
    }).collect();
    list.sort_by_key(|t| t.name.to_lowercase());
    list
}

/// the name shown for a theme of one of the sources
pub fn theme_name(id: &str) -> Option<String> {
    let (source, file) = split_id(id)?;
    file_name_of(source, file)
}

// None when the file cannot be read as a theme
fn file_name_of(source: Source, file: &str) -> Option<String> {
    let stem = || file.strip_suffix(".palette").unwrap_or(file).to_string();
    match source.format(file) {
        Format::Tilix => Some(parse_tilix(&read(source, file)?)?.0),
        Format::Ptyxis => parse(&read(source, file)?).map(|(n, ..)| if n.is_empty() { stem() } else { n }),
        Format::Ghostty => Some(file.to_string()),
    }
}

fn ghostty_color(value: &str) -> Option<RGBA> {
    let v = value.trim().trim_matches('"');
    let hex = v.len() == 6 && v.chars().all(|c| c.is_ascii_hexdigit());
    RGBA::parse(if hex { format!("#{v}") } else { v.to_string() }.as_str()).ok()
}

fn parse_ghostty(text: &str) -> Option<Variant> {
    let (mut fg, mut bg, mut cursor) = (None, None, None);
    let mut palette: [Option<RGBA>; 16] = [None; 16];
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        match key.trim() {
            "foreground" => fg = ghostty_color(value),
            "background" => bg = ghostty_color(value),
            "cursor-color" => cursor = ghostty_color(value),
            "palette" => {
                let Some((i, c)) = value.split_once('=') else { continue };
                if let (Ok(i), Some(c)) = (i.trim().parse::<usize>(), ghostty_color(c)) {
                    if let Some(slot) = palette.get_mut(i) {
                        *slot = Some(c);
                    }
                }
            }
            _ => {}
        }
    }
    let palette: Vec<RGBA> = palette.iter().flatten().copied().collect();
    Some(Variant { fg: Some(fg?), bg: Some(bg?), cursor, palette: if palette.len() == 16 { palette } else { Vec::new() } })
}

/// (name, colors, whether text and background follow the system style)
fn parse_tilix(text: &str) -> Option<(String, Variant, bool)> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let color = |key: &str| v.get(key).and_then(|c| c.as_str()).and_then(|c| RGBA::parse(c).ok());
    let palette: Vec<RGBA> = v.get("palette").and_then(|p| p.as_array())
        .map(|a| a.iter().filter_map(|c| c.as_str().and_then(|c| RGBA::parse(c).ok())).collect())
        .unwrap_or_default();
    let name: String = v.get("name").and_then(|n| n.as_str()).map(str::trim).filter(|n| !n.is_empty())?
        .chars().take(100).collect();
    let follow = v.get("use-theme-colors").and_then(|b| b.as_bool()).unwrap_or(false);
    let colors = Variant {
        fg: color("foreground-color"),
        bg: color("background-color"),
        cursor: color("cursor-background-color"),
        palette: if palette.len() == 16 { palette } else { Vec::new() },
    };
    if !follow && (colors.fg.is_none() || colors.bg.is_none()) {
        return None;
    }
    Some((name, colors, follow))
}

fn luminance(c: &RGBA) -> f32 {
    0.2126 * c.red() + 0.7152 * c.green() + 0.0722 * c.blue()
}

// a theme with a single variant: its background tells the style
fn single(mut v: Variant) -> Scheme {
    let dark = v.bg.as_ref().is_some_and(|b| luminance(b) < 0.5);
    if v.palette.is_empty() {
        let g = gnome();
        v.palette = if dark { g.dark.palette } else { g.light.palette };
    }
    Scheme { light: v.clone(), dark: v, force_dark: Some(dark) }
}

fn load(source: Source, file: &str) -> Option<Scheme> {
    let text = read(source, file)?;
    match source.format(file) {
        Format::Ghostty => return Some(single(parse_ghostty(&text)?)),
        Format::Ptyxis => {
            return parse(&text).map(|(_, scheme)| scheme);
        }
        Format::Tilix => {}
    }
    let (_, v, follow) = parse_tilix(&text)?;
    if !follow {
        return Some(single(v));
    }
    // text and background of the system style, with the colors of the theme
    let mut s = nautilus();
    if !v.palette.is_empty() {
        s.light.palette = v.palette.clone();
        s.dark.palette = v.palette;
    }
    Some(s)
}

fn hex(c: &RGBA) -> String {
    format!("#{:02x}{:02x}{:02x}", (c.red() * 255.0).round() as u8,
            (c.green() * 255.0).round() as u8, (c.blue() * 255.0).round() as u8)
}

/// the file name of a theme of the user, from its name
pub fn theme_file(name: &str) -> String {
    let f: String = name.trim().chars().map(|c| if c == '/' || c.is_control() { '-' } else { c }).collect();
    let f: String = f.trim_start_matches('.').trim().chars().take(100).collect();
    if f.is_empty() { f } else { format!("{f}.json") }
}

/// whether a theme of the user, other than the one in file `except`, has this name
pub fn mine_taken(name: &str, except: Option<&str>) -> bool {
    let (name, file) = (name.trim().to_lowercase(), theme_file(name));
    themes(Source::Mine).iter()
        .filter(|t| split_id(&t.id).map(|(_, f)| f) != except)
        .any(|t| t.name.to_lowercase() == name || t.id.ends_with(&format!(":{file}")))
}

// fg or bg None: text and background follow the system style
fn theme_json(name: &str, v: &Variant) -> String {
    let follow = v.fg.is_none() || v.bg.is_none();
    let mut j = serde_json::json!({
        "name": name.trim(),
        "comment": "Made with ShellMan",
        "use-theme-colors": follow,
        "foreground-color": v.fg.as_ref().map(hex),
        "background-color": v.bg.as_ref().map(hex),
        "palette": v.palette.iter().take(16).map(hex).collect::<Vec<_>>(),
    });
    if let (Some(c), Some(bg)) = (&v.cursor, &v.bg) {
        j["use-cursor-color"] = true.into();
        j["cursor-background-color"] = hex(c).into();
        j["cursor-foreground-color"] = hex(bg).into();
    }
    serde_json::to_string_pretty(&j).unwrap_or_default() + "\n"
}

/// writes a theme of the user; replaces: the file of the theme being edited
pub fn save_mine(name: &str, v: &Variant, replaces: Option<&str>) -> Option<String> {
    let file = theme_file(name);
    if !valid_file(&file) {
        return None;
    }
    let dir = mine_dir();
    std::fs::create_dir_all(&dir).ok()?;
    let tmp = dir.join(format!(".{file}.tmp"));
    std::fs::write(&tmp, theme_json(name, v)).ok()?;
    std::fs::rename(&tmp, dir.join(&file)).ok()?;
    if let Some(old) = replaces.filter(|o| *o != file && valid_file(o)) {
        let _ = std::fs::remove_file(dir.join(old));
    }
    Some(format!("{}{file}", Source::Mine.prefix()))
}

/// name and colors of a theme file of Tilix, Black Box, Ptyxis or Ghostty;
/// for a Ptyxis palette, its variant for the current style
pub fn read_theme_file(path: &Path) -> Option<(String, Variant)> {
    let text = read_file(path, THEME_MAX)?;
    if let Some((name, scheme)) = parse(&text) {
        let stem = path.file_stem()?.to_string_lossy().trim().to_string();
        let (light, dark) = (scheme.light.clone(), scheme.dark.clone());
        let v = if resolve(scheme).1 { dark } else { light };
        return Some((if name.is_empty() { stem } else { name }, v));
    }
    if let Some((name, v, follow)) = parse_tilix(&text) {
        let v = if follow { Variant { fg: None, bg: None, cursor: None, palette: v.palette } } else { v };
        return Some((name, v));
    }
    let v = parse_ghostty(&text)?;
    let stem = path.file_stem()?.to_string_lossy().trim().to_string();
    Some((stem, v))
}

pub fn delete_mine(file: &str) {
    if valid_file(file) {
        let _ = std::fs::remove_file(mine_dir().join(file));
    }
}

/// every color of a theme as shown now, as a starting point for a theme of the user
pub fn editable(id: &str) -> Variant {
    let (mut v, dark) = resolve(scheme_for(id, &mut Vec::new()));
    let g = gnome();
    let g = if dark { g.dark } else { g.light };
    v.fg = v.fg.or(g.fg);
    v.bg = v.bg.or(g.bg);
    v.cursor = v.cursor.or(v.fg);
    if v.palette.len() != 16 {
        v.palette = g.palette;
    }
    v
}

fn nautilus() -> Scheme {
    let g = gnome();
    let clear = |v: Variant| Variant { fg: None, bg: None, cursor: None, palette: v.palette };
    Scheme { light: clear(g.light), dark: clear(g.dark), force_dark: None }
}

thread_local! {
    static WATCH: RefCell<Vec<gio::Settings>> = const { RefCell::new(Vec::new()) };
}

fn scheme_for(id: &str, watch: &mut Vec<gio::Settings>) -> Scheme {
    match id {
        FOLLOW_TERMINAL => match default_terminal() {
            Some(Terminal::Ptyxis) => ptyxis(watch),
            Some(Terminal::GnomeTerminal) => gnome_terminal(watch),
            None => None,
        }.unwrap_or_else(nautilus),
        NAUTILUS => nautilus(),
        id => match split_id(id) {
            Some((source, file)) => load(source, file),
            None => builtin_scheme(id),
        }.unwrap_or_else(nautilus),
    }
}

fn scheme() -> Scheme {
    let mut watch = Vec::new();
    let scheme = scheme_for(&settings::with(|s| s.palette.clone()), &mut watch);
    for w in &watch {
        w.connect_changed(None, |_, _| guard("terminal colors", (), || {
            glib::idle_add_local_once(|| guard("terminal colors", (), crate::panel::restyle_all));
        }));
        // GSettings only reports keys read after connecting
        if let Some(schema) = w.settings_schema() {
            for key in schema.list_keys() {
                let _ = w.value(&key);
            }
        }
    }
    WATCH.with(|c| c.replace(watch));
    scheme
}

/// Colors to use now; dark: whether the variant is dark
pub fn current() -> (Variant, bool) {
    resolve(scheme())
}

/// Colors a palette id would give now, for previews
pub fn preview(id: &str) -> Variant {
    resolve(scheme_for(id, &mut Vec::new())).0
}

fn resolve(scheme: Scheme) -> (Variant, bool) {
    let system_dark = adw::StyleManager::default().is_dark();
    let dark = match settings::with(|s| s.style.clone()).as_str() {
        "dark" => true,
        "light" => false,
        _ => scheme.force_dark.unwrap_or(system_dark),
    };
    let mut v = if dark { scheme.dark } else { scheme.light };
    if (v.fg.is_none() || v.bg.is_none()) && dark != system_dark {
        // the file view has the other style: it cannot show through
        let g = gnome();
        let g = if dark { g.dark } else { g.light };
        v.fg = v.fg.or(g.fg);
        v.bg = v.bg.or(g.bg);
    }
    (v, dark)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ghostty_theme_is_read() {
        let mut text = String::from("# comment\nbackground = 1d1f21\nforeground = #c5c8c6\ncursor-color = #ffffff\n");
        for i in 0..16 {
            text.push_str(&format!("palette = {i}=#0000{i:02x}\n"));
        }
        let v = parse_ghostty(&text).unwrap();
        assert_eq!(v.bg, RGBA::parse("#1d1f21").ok());
        assert_eq!(v.fg, RGBA::parse("#c5c8c6").ok());
        assert_eq!(v.palette.len(), 16);
        assert_eq!(v.palette[15], RGBA::parse("#00000f").unwrap());
        assert!(parse_ghostty("foreground = #ffffff").is_none());
    }

    #[test]
    fn installed_themes_parse() {
        for source in Source::ALL {
            for t in themes(source) {
                let (_, file) = split_id(&t.id).unwrap();
                assert!(load(source, file).is_some_and(|s| s.dark.palette.len() == 16), "{}", t.id);
            }
        }
    }

    #[test]
    fn single_ptyxis_palette() {
        let mut text = String::from("[Palette]\nName=One\nBackground=#1F1F28\nForeground=#DCD7BA\n");
        for i in 0..16 {
            text.push_str(&format!("Color{i}=#0000{i:02x}\n"));
        }
        let (name, s) = parse(&text).unwrap();
        assert_eq!(name, "One");
        assert_eq!(s.force_dark, Some(true));
        assert_eq!(s.dark.palette.len(), 16);
    }

    #[test]
    fn damaged_theme_files_are_skipped() {
        let dir = std::env::temp_dir().join(format!("shellman-themes-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let deep = "[".repeat(100_000);
        let cases: [(&str, &[u8]); 6] = [
            ("broken.json", b"{\"name\": \"x\", \"palette\": [\"#000\""),
            ("deep.json", deep.as_bytes()),
            ("binary.json", &[0xff, 0xfe, 0x00, 0x7b]),
            ("empty", b""),
            ("garbage", b"palette = 99999999999999999999=#zzz\nbackground = \nforeground"),
            ("half.palette", b"[Palette]\nName=Half\n[Light]\nForeground=#000000\n"),
        ];
        for (name, data) in cases {
            let path = dir.join(name);
            std::fs::write(&path, data).unwrap();
            assert!(read_theme_file(&path).is_none(), "{name}");
        }
        std::fs::write(dir.join("huge"), vec![b'#'; THEME_MAX as usize + 1]).unwrap();
        assert!(read_file(&dir.join("huge"), THEME_MAX).is_none());
        assert!(read_file(Path::new("/dev/zero"), THEME_MAX).is_none());
        assert!(read_file(&dir, THEME_MAX).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn tilix_scheme_is_read() {
        let json = r##"{"name": "Test", "use-theme-colors": false, "foreground-color": "#f8f8f2",
            "background-color": "#1e1f29", "palette": ["#000000", "#ff5555", "#50fa7b", "#f1fa8c",
            "#bd93f9", "#ff79c6", "#8be9fd", "#bbbbbb", "#555555", "#ff5555", "#50fa7b", "#f1fa8c",
            "#bd93f9", "#ff79c6", "#8be9fd", "#ffffff"]}"##;
        let (name, v, follow) = parse_tilix(json).unwrap();
        assert_eq!(name, "Test");
        assert!(!follow);
        assert_eq!(v.bg, RGBA::parse("#1e1f29").ok());
        assert_eq!(v.palette.len(), 16);
        assert!(parse_tilix(r#"{"name": "No colors"}"#).is_none());
        assert!(parse_tilix(r#"{"name": "System", "use-theme-colors": true}"#).is_some_and(|t| t.2));
    }

    #[test]
    fn builtin_palettes_are_complete() {
        for (id, text) in BUILTIN {
            let (name, s) = parse(text).unwrap_or_else(|| panic!("{id}"));
            assert!(!name.is_empty(), "{id}");
            assert_eq!(s.light.palette.len(), 16, "{id}");
            assert_eq!(s.dark.palette.len(), 16, "{id}");
        }
    }

    #[test]
    fn theme_terminal_css() {
        let css = "a { color: red; }\nterminal-window .terminal-screen {\n  background-color: #300A24;\n  color: white; }\n";
        let block = css_block(css, "terminal-window .terminal-screen").unwrap();
        assert_eq!(css_color(block, "background-color"), RGBA::parse("#300a24").ok());
        assert_eq!(css_color(block, "color"), RGBA::parse("white").ok());
        assert_eq!(css_block(css, "nothing"), None);
    }

    #[test]
    fn yaru_terminal_colors() {
        if !Path::new("/usr/share/themes/Yaru-dark/gtk-3.0").is_dir() {
            return;
        }
        let (fg, bg) = gtk3_terminal_colors("Yaru-dark", true).unwrap();
        assert_eq!(bg, RGBA::parse("#300a24").unwrap());
        assert_eq!(fg, RGBA::parse("white").unwrap());
        assert!(gtk3_terminal_colors("../Yaru-dark", true).is_none());
    }

    #[test]
    fn terminal_list_entries() {
        let dir = std::env::temp_dir().join(format!("shellman-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("xdg-terminals.list");
        std::fs::write(&f, "# comment\n\n-org.gnome.Ptyxis.desktop\nno.such.terminal.desktop\n").unwrap();
        assert_eq!(first_entry(&f), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

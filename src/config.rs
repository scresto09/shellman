use crate::N_;

pub const MIN_ROWS: i32 = 3;
pub const MIN_FILES_HEIGHT: i32 = 120;
pub const SCROLLBACK_LINES: i64 = 10_000;
pub const BUTTON_ICON: &str = "utilities-terminal-symbolic";
pub const ACTIONS: &[(&str, &str)] = &[
    ("toggle", N_!("Show or Hide the Terminal")),
    ("copy", N_!("Copy")),
    ("paste", N_!("Paste")),
    ("find", N_!("Find…")),
    ("find-next", N_!("Find Next")),
    ("find-previous", N_!("Find Previous")),
    ("insert", N_!("Insert Selected Files")),
    ("clear", N_!("Clear All")),
    ("zoom-in", N_!("Zoom In")),
    ("zoom-out", N_!("Zoom Out")),
    ("zoom-reset", N_!("Normal Size")),
    ("more-lines", N_!("More Lines")),
    ("fewer-lines", N_!("Fewer Lines")),
    ("wider", N_!("Widen Terminal")),
    ("narrower", N_!("Narrow Terminal")),
    ("prev-prompt", N_!("Previous Prompt")),
    ("next-prompt", N_!("Next Prompt")),
    ("split-right", N_!("Split Right")),
    ("split-down", N_!("Split Down")),
    ("split-auto", N_!("Split Automatically")),
    ("close", N_!("Close Terminal")),
    ("goto-tab-dir", N_!("Terminal: Go to the Nautilus Folder")),
    ("follow", N_!("Follow Nautilus Folder")),
    ("show-in-nautilus", N_!("Nautilus: Go to the Terminal Folder")),
    ("focus-left", N_!("Go to Left Terminal")),
    ("focus-right", N_!("Go to Right Terminal")),
    ("focus-up", N_!("Go to Terminal Above")),
    ("focus-down", N_!("Go to Terminal Below")),
];

pub const SHORTCUT_GROUPS: &[(&str, &[&str])] = &[
    (N_!("Terminal"), &["toggle", "copy", "paste", "insert", "clear", "prev-prompt", "next-prompt",
                    "zoom-in", "zoom-out", "zoom-reset", "more-lines", "fewer-lines"]),
    (N_!("Search"), &["find", "find-next", "find-previous"]),
    (N_!("Folder"), &["goto-tab-dir", "show-in-nautilus", "follow"]),
    (N_!("Splits"), &["split-right", "split-down", "split-auto", "close", "wider", "narrower",
                      "focus-left", "focus-right", "focus-up", "focus-down"]),
];

pub const DEFAULT_SHORTCUTS: &[(&str, &str)] = &[
    ("F4", "toggle"),
    ("<Control><Shift>c", "copy"),
    ("<Control><Shift>v", "paste"),
    ("<Control><Shift>s", "insert"),
    ("<Alt><Shift>equal", "split-right"),
    ("<Alt><Shift>KP_Add", "split-right"),
    ("<Alt><Shift>minus", "split-down"),
    ("<Alt><Shift>KP_Subtract", "split-down"),
    ("<Alt><Shift>d", "split-auto"),
    ("<Control><Shift>w", "close"),
    ("<Control><Shift>f", "find"),
    ("<Control><Shift>g", "find-next"),
    ("<Control><Shift>h", "find-previous"),
    ("<Control><Shift>d", "goto-tab-dir"),
    ("<Control><Shift>n", "show-in-nautilus"),
    ("<Control><Shift>k", "clear"),
    ("<Control>equal", "zoom-in"),
    ("<Control><Shift>plus", "zoom-in"),
    ("<Control>KP_Add", "zoom-in"),
    ("<Control>minus", "zoom-out"),
    ("<Control>KP_Subtract", "zoom-out"),
    ("<Control>0", "zoom-reset"),
    ("<Control>KP_0", "zoom-reset"),
    ("<Control><Shift>0", "zoom-reset"),
    ("<Control><Shift>Page_Up", "more-lines"),
    ("<Control><Shift>Page_Down", "fewer-lines"),
    ("<Control><Alt><Shift>Right", "wider"),
    ("<Control><Alt><Shift>Left", "narrower"),
    ("<Control><Shift>Up", "prev-prompt"),
    ("<Control><Shift>Down", "next-prompt"),
    ("<Alt><Shift>Left", "focus-left"),
    ("<Alt><Shift>Right", "focus-right"),
    ("<Alt><Shift>Up", "focus-up"),
    ("<Alt><Shift>Down", "focus-down"),
];
pub const MIN_PANE_WIDTH: i32 = 160;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_is_in_one_shortcut_group() {
        for (action, _) in ACTIONS {
            let n = SHORTCUT_GROUPS.iter().filter(|(_, a)| a.contains(action)).count();
            assert_eq!(n, 1, "{action}");
        }
        for (_, actions) in SHORTCUT_GROUPS {
            for a in *actions {
                assert!(ACTIONS.iter().any(|(x, _)| x == a), "{a}");
            }
        }
    }
}

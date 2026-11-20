# ShellMan

A terminal at the bottom of Nautilus (GNOME Files) windows.
Home page: https://scresto.pages-perso.free.fr/shellman/

![ShellMan in Nautilus](https://scresto.pages-perso.free.fr/shellman/img/screenshot2.png)

- **F4** shows the terminal, gives it the keyboard, then hides it.
- The shell **follows the displayed folder**, and Nautilus follows `cd` in the terminal
  (bash, zsh, fish, elvish, nushell), without typing anything into it. Either way can be
  turned off, for all terminals or one.
- The **selected files** are in `$SEL`: `cp "${SEL[@]}" /tmp`.
  **Ctrl+Shift+S** inserts their paths, quoted.
- **One terminal per tab**, which can be split (**Alt+Shift+=**, **Alt+Shift+-**).
- **Clickable names** in the output of `ls`: right-click to open a file or go to a folder.
- **Web addresses** in the output open with **Ctrl+click** (or right-click). Files dropped on
  the terminal are inserted, quoted.
- A **notification** when a command of more than 10 seconds ends while the terminal is hidden,
  and the terminal button turns to the accent color while a command runs behind it.
- At the prompt: **Ctrl+Shift+↑/↓** jumps between prompts, **Alt+click** moves the cursor,
  **Ctrl+Shift+K** clears the terminal and its history.
- **Search** in the terminal output (**Ctrl+Shift+F**, then **Ctrl+Shift+G** / **Ctrl+Shift+H**).
- The **colors of your terminal** (GNOME Terminal or Ptyxis), a built-in theme, the themes
  of **Black Box**, **Ghostty** and **Tilix** and the palettes added to **Ptyxis**, when they are
  installed, or **your own themes**, made from any of them or imported from a theme file of
  these terminals (saved as Tilix files in `~/.config/shellman/themes/`).
- An optional **background image**, with opacity and blur, or a **transparent terminal**.
- **Zoom** with **Ctrl++** / **Ctrl+-** / **Ctrl+0**; more or fewer lines with
  **Ctrl+Shift+Page↑/↓**.
- Shortcuts, colors, font and background can be changed in **Terminal ▾ › Preferences…**.
- If Nautilus crashes while the terminal is set up, ShellMan stays off at the next start:
  right-click an empty area of the folder (with no file selected), then choose **Re-enable ShellMan**.

Tested with Nautilus 46 to 51. ShellMan changes Nautilus' window, which the
extension API does not really allow: a Nautilus update can break it.

## Installation

Each [release](https://github.com/scresto09/shellman/releases) has a `.deb`
(Ubuntu 24.04 and later, Debian 13) and a `.rpm` (Fedora, RHEL 10, openSUSE).

```bash
sudo apt install ./shellman_*.deb                          # Debian, Ubuntu
sudo dnf install ./shellman-*.rpm                          # Fedora, RHEL
sudo zypper install --allow-unsigned-rpm ./shellman-*.rpm  # openSUSE
nautilus -q
```

From source (Rust, GTK 4, libadwaita and the Nautilus extension headers):

```bash
make && sudo make install
```

Tests: see [TESTING.md](TESTING.md).

## License

GPL-3.0-or-later, see [LICENSE](LICENSE).

The color palettes in `palettes/` come from [Ptyxis](https://gitlab.gnome.org/chergert/ptyxis)
(GPL-3.0-or-later), unchanged.

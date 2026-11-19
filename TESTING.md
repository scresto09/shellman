# Testing ShellMan

## Unit tests

```bash
cargo test
```

Some tests run the shells installed on the machine (bash, zsh, fish, dash, ksh,
tcsh, elvish, nu…) to check that inserted file names are read back exactly;
missing shells are skipped.

## Nautilus in a container

`packaging/nautilus-test.sh IMAGE PACKAGE` installs a built package in a clean
container, starts Nautilus under Xvfb (or headless mutter) and checks that the
terminal starts its shell:

```bash
packaging/build.sh deb
packaging/nautilus-test.sh ubuntu:24.04 shellman_0.2.0-1_amd64.deb
```

## Environment variables

Read by the extension, for debugging and tests:

| Variable | Effect |
|---|---|
| `SHELLMAN_QUIET` | no log lines on stderr |
| `SHELLMAN_DUMP` | writes the widget tree of the window to `~/shellman-tree.txt` |
| `SHELLMAN_TEST_SHOW` | opens the terminal right after the window |
| `SHELLMAN_TEST_SPLIT` | then splits it |
| `SHELLMAN_TEST_INPUT=cmd1;cmd2` | then types these lines, one every 700 ms |
| `SHELLMAN_TEST_SNAPSHOT=file.png` | saves a render of the window (with alpha), and logs the focused widget |
| `SHELLMAN_TEST_SNAPSHOT_DELAY=ms` | delay before the snapshot (default 3000) |
| `SHELLMAN_TEST_SEE_THROUGH` | allows the see-through option without a compositor (Xvfb) |

Example, in a container with Nautilus and Xvfb:

```bash
SHELLMAN_TEST_SHOW=1 SHELLMAN_TEST_INPUT='cd /usr/share' \
SHELLMAN_TEST_SNAPSHOT=/tmp/win.png SHELLMAN_TEST_SNAPSHOT_DELAY=5000 \
xvfb-run -a nautilus /tmp
```

Set by the extension for the shell (shell integration, see `src/shell.rs`):
`SHELLMAN`, `SHELLMAN_SEL`, `SHELLMAN_CD`, `SHELLMAN_STATE`,
`SHELLMAN_LS_LINKS`, and `SHELLMAN_ZDOTDIR` for zsh.

Build time: `SHELLMAN_LOCALEDIR` (installed translations, set by the Makefile).

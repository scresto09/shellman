# The version comes from build.sh: rpmbuild --define "shellman_version 0.1.0"

# rustc release builds carry no debug information
%global debug_package %{nil}

Name:           shellman
Version:        %{shellman_version}
Release:        %{?shellman_release}%{!?shellman_release:1}%{?dist}
Summary:        Embedded terminal for Nautilus (GNOME Files)
License:        GPL-3.0-or-later
URL:            https://github.com/scresto09/shellman
Source0:        shellman-%{version}.tar.gz

%if 0%{?suse_version}
BuildRequires:  AppStream
%else
BuildRequires:  appstream
%endif
BuildRequires:  gcc
BuildRequires:  gettext
BuildRequires:  make
BuildRequires:  pkgconfig(gtk4) >= 4.14
BuildRequires:  pkgconfig(libadwaita-1) >= 1.5
BuildRequires:  pkgconfig(libnautilus-extension-4)
BuildRequires:  pkgconfig(vte-2.91-gtk4)
Requires:       nautilus >= 43

%description
ShellMan adds a terminal at the bottom of Nautilus windows, one per tab. It
follows the displayed folder, the files selected in Nautilus are available in
the SEL variable, and terminals can be split. Press F4 to show or hide it.

%prep
%setup -q

%build
make PREFIX=%{_prefix}

%install
make install PREFIX=%{_prefix} DESTDIR=%{buildroot}
%find_lang %{name}

%check
make check

# Nautilus loads its extensions at startup: tell the users running it to
# restart it. Never quit it, it could interrupt a file operation.
# Same code in debian/postinst.
%posttrans
notify_restart() {
	[ / -ef /proc/1/root ] || return 0
	if command -v systemd-detect-virt >/dev/null 2>&1; then
		systemd-detect-virt -q --container && return 0
	fi
	[ -e /.dockerenv ] || [ -e /run/.containerenv ] && return 0
	for cmd in runuser timeout pgrep notify-send; do
		command -v $cmd >/dev/null 2>&1 || return 0
	done
	for bus in /run/user/*/bus; do
		[ -S "$bus" ] || continue
		uid=$(basename "$(dirname "$bus")")
		pid=$(pgrep -o -u "$uid" -x nautilus) || continue
		user=$(id -nu "$uid" 2>/dev/null) || continue
		env=$(tr '\0' '\n' < "/proc/$pid/environ" 2>/dev/null) || env=
		lang=
		for var in LANGUAGE LC_ALL LC_MESSAGES LANG; do
			lang=$(echo "$env" | sed -n "s/^$var=//p")
			[ -n "$lang" ] && break
		done
		case $lang in
		  fr*)
		    title="ShellMan installé : redémarrez Nautilus"
		    text="Exécutez nautilus -q ou fermez puis rouvrez votre session."
		    ;;
		  *)
		    title="ShellMan installed: restart Nautilus"
		    text="Run nautilus -q, or log out and back in."
		    ;;
		esac
		timeout 5 runuser -u "$user" -- env DBUS_SESSION_BUS_ADDRESS="unix:path=$bus" \
			notify-send -a ShellMan -i utilities-terminal "$title" "$text" \
			>/dev/null 2>&1 || true
	done
}
notify_restart || true

%files -f %{name}.lang
%license LICENSE
%doc README.md
%{_libdir}/nautilus/extensions-4/libnautilus-shellman.so
%{_datadir}/metainfo/io.github.scresto09.ShellMan.metainfo.xml

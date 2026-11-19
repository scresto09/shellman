#!/bin/sh
# Extract translatable strings into po/shellman.pot and merge them into each po/<lang>.po
set -e
cd "$(dirname "$0")/.."
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
FILES=$(grep -rlE --include='*.rs' "\b(n?tr\(|N_!\()" src | sort)
for f in $FILES; do
    mkdir -p "$TMP/$(dirname "$f")"
    sed 's/N_!(/N_(/g' "$f" > "$TMP/$f"
done
(cd "$TMP" && xgettext --language=C --from-code=UTF-8 --keyword=tr --keyword=ntr:1,2 --keyword=N_ \
    --package-name=shellman --package-version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$OLDPWD/Cargo.toml" | head -1)" \
    --msgid-bugs-address=https://github.com/scresto09/shellman/issues \
    --add-comments=TRANSLATORS --sort-by-file -o "$OLDPWD/po/shellman.pot" $FILES)
xgettext --join-existing --from-code=UTF-8 -o po/shellman.pot data/*.metainfo.xml
for lang in $(grep -v '^#' po/LINGUAS); do
    if [ -f "po/$lang.po" ]; then
        msgmerge --quiet --update --backup=none "po/$lang.po" po/shellman.pot
    else
        msginit --no-translator --locale="$lang.UTF-8" -i po/shellman.pot -o "po/$lang.po"
    fi
done

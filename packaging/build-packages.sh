#!/usr/bin/env bash
# Builds the release tarball and the .deb from already-built binaries.
#
#   packaging/build-packages.sh VERSION BIN_DIR RCLONE OUT_DIR
#
# VERSION  e.g. 0.1.0 (no leading v)
# BIN_DIR  folder holding `bufonidae` and `bufonidae-desktop`
# RCLONE   the rclone binary to ship (the tested release)
# OUT_DIR  where the packages and SHA256SUMS go
#
# The build is deterministic: file order, owners and times are fixed from the
# last commit, so the same inputs give the same bytes.
set -euo pipefail

[ "$#" -eq 4 ] || { sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'; exit 2; }
version=$1 bin_dir=$2 rclone=$3 out=$4

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
for f in "$bin_dir/bufonidae" "$bin_dir/bufonidae-desktop" "$rclone"; do
    [ -x "$f" ] || { echo "missing or not executable: $f" >&2; exit 1; }
done

epoch=${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct 2>/dev/null || echo 0)}
# dpkg-deb stamps its archive members from this variable, not from file times.
export SOURCE_DATE_EPOCH=$epoch
mkdir -p "$out"
out=$(cd "$out" && pwd)
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT

# One layout, used by both packages: <prefix>/bin, libexec, share.
fill() {
    local prefix=$1
    install -D -m 0755 "$bin_dir/bufonidae" "$prefix/bin/bufonidae"
    install -D -m 0755 "$bin_dir/bufonidae-desktop" "$prefix/bin/bufonidae-desktop"
    # The rclone the app was tested with; found beside the binary, see
    # bufonidae_rclone::locate.
    install -D -m 0755 "$rclone" "$prefix/libexec/bufonidae/rclone"
    install -D -m 0644 packaging/bufonidae.desktop "$prefix/share/applications/bufonidae.desktop"
    install -D -m 0644 packaging/bufonidae.svg "$prefix/share/icons/hicolor/scalable/apps/bufonidae.svg"
    local doc="$prefix/share/doc/bufonidae"
    install -D -m 0644 README.md "$doc/README.md"
    install -D -m 0644 LICENSE "$doc/LICENSE"
    install -D -m 0644 SECURITY.md "$doc/SECURITY.md"
    install -D -m 0644 CHANGELOG.md "$doc/CHANGELOG.md"
    install -D -m 0644 docs/COMPATIBILITY.md "$doc/COMPATIBILITY.md"
    install -D -m 0644 docs/RECOVERY.md "$doc/RECOVERY.md"
    install -D -m 0644 packaging/THIRD-PARTY.md "$doc/THIRD-PARTY.md"
    find "$prefix" -exec touch -h -d "@$epoch" {} +
}

# ---- tarball -------------------------------------------------------------
name="bufonidae-$version-linux-x86_64"
fill "$stage/$name"
tar --sort=name --owner=0 --group=0 --numeric-owner --mtime="@$epoch" \
    -C "$stage" -cf - "$name" | gzip -n -9 >"$out/$name.tar.gz"

# ---- .deb ----------------------------------------------------------------
deb_root="$stage/deb"
fill "$deb_root/usr"
mkdir -p "$deb_root/DEBIAN"
# `Installed-Size` is in KiB.
size=$(du -sk --apparent-size "$deb_root/usr" | cut -f1)
cat >"$deb_root/DEBIAN/control" <<CONTROL
Package: bufonidae
Version: $version
Architecture: amd64
Maintainer: Bufonidae maintainers <noreply@users.noreply.github.com>
Installed-Size: $size
Section: utils
Priority: optional
Homepage: https://github.com/Lucas-BRT/bufonidae
Depends: libc6 (>= 2.39), libxcb1, libxkbcommon0, libxkbcommon-x11-0, libwayland-client0, libfontconfig1, libfreetype6, libvulkan1
Recommends: mesa-vulkan-drivers, gnome-keyring | kwalletmanager
Description: Spread files across several Mega accounts
 Bufonidae pools several Mega.nz accounts you own into one logical folder,
 storing each file whole on the account with the most room and optionally
 keeping copies on different accounts. It ships the rclone release it is
 tested with and never touches the rclone of the system.
 .
 Development software: not released, keep your own backups.
CONTROL
find "$deb_root" -exec touch -h -d "@$epoch" {} +
dpkg-deb --root-owner-group -Zxz --build "$deb_root" "$out/bufonidae_${version}_amd64.deb" >/dev/null

# ---- checksums -----------------------------------------------------------
(cd "$out" && sha256sum "$name.tar.gz" "bufonidae_${version}_amd64.deb" >SHA256SUMS)
echo "built in $out:"
ls -l "$out"

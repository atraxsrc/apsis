name := 'apsis'
appid := 'io.github.atraxsrc.Apsis'

rootdir := ''
prefix := '/usr'

metainfo := appid + '.metainfo.xml'
desktop := appid + '.desktop'
# The app launcher's entry (`apsis --window`); the applet's entry above is panel-only.
launcher := appid + '.Window.desktop'

# Installation paths
base-dir := absolute_path(clean(rootdir / prefix))
cargo-target-dir := env('CARGO_TARGET_DIR', 'target')
metainfo-dst := base-dir / 'share' / 'metainfo' / metainfo
bin-dst := base-dir / 'bin' / name
desktop-dst := base-dir / 'share' / 'applications' / desktop
launcher-dst := base-dir / 'share' / 'applications' / launcher
icons-dir := base-dir / 'share' / 'icons' / 'hicolor'
icon-dst := icons-dir / 'scalable' / 'apps' / appid + '.svg'
icon-symbolic-dst := icons-dir / 'symbolic' / 'apps' / appid + '-symbolic.svg'
# The man page, written in roff (docs/apsis.1), installed gzipped. `-n` leaves out the name and
# time stamp, so the file is the same on every build (lintian checks this).
man-dst := base-dir / 'share' / 'man' / 'man1' / name + '.1.gz'

# Privileged helper (Phase 4): the binary, D-Bus activation and bus policy, systemd unit, and
# polkit actions. `libexec-path` is where the helper lives at run time (written into the
# activation file and unit); the *-dst paths include rootdir.
helper := 'apsis-helper'
helper-bus := appid + '.Helper'
helper-res := 'resources' / 'helper'
libexec-path := prefix / 'libexec'
helper-dst := base-dir / 'libexec' / helper
dbus-service-dst := base-dir / 'share' / 'dbus-1' / 'system-services' / helper-bus + '.service'
dbus-policy-dst := base-dir / 'share' / 'dbus-1' / 'system.d' / helper-bus + '.conf'
systemd-unit-dst := base-dir / 'lib' / 'systemd' / 'system' / helper + '.service'
polkit-dst := base-dir / 'share' / 'polkit-1' / 'actions' / appid + '.policy'
# systemd and the bus only pick up new files when told; skipped when packaging into rootdir.
reload-system := "if [ -z '" + rootdir + "' ]; then systemctl daemon-reload && busctl call org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus ReloadConfig; fi"

# Default recipe which runs `just build-release`
default: build-release

# Runs `cargo clean`
clean:
    cargo clean

# Removes vendored dependencies
clean-vendor:
    rm -rf .cargo vendor vendor.tar

# `cargo clean` and removes vendored dependencies
clean-dist: clean clean-vendor

# Compiles with debug profile
build-debug *args:
    cargo build {{args}}

# Compiles with release profile
build-release *args: (build-debug '--release' args)

# Compiles release profile with vendored dependencies. vendor/ and .cargo/ are removed again
# afterwards, even if the build fails, so plain cargo builds never pick up the vendored sources.
build-vendored *args:
    #!/usr/bin/env bash
    set -euo pipefail
    rm -rf vendor .cargo
    trap 'rm -rf vendor .cargo' EXIT
    tar pxf vendor.tar
    cargo build --release --frozen {{args}}

# Runs a clippy check
check *args:
    cargo clippy --workspace --all-features {{args}} -- -W clippy::pedantic

# Runs a clippy check with JSON message format
check-json: (check '--message-format=json')

# Native backend (Phase 5): a 128 MB ext4 image for its tests, made without root. The user
# mounts it (the one step needing root), then `just test-ext4` runs the tests on it.
ext4-dir := cargo-target-dir / 'native-ext4'

# Builds the ext4 test image and prints the mount command
ext4-image:
    mkdir -p {{ext4-dir}}/mnt
    truncate -s 128M {{ext4-dir}}/ext4.img
    /usr/sbin/mkfs.ext4 -F -q -L apsis-test -E root_owner=$(id -u):$(id -g) {{ext4-dir}}/ext4.img
    @echo "now: sudo mount -o loop,nosuid,nodev {{ext4-dir}}/ext4.img {{ext4-dir}}/mnt"

# Runs the native backend's tests on the mounted ext4 image too
test-ext4:
    APSIS_EXT4_MNT="$(realpath {{ext4-dir}}/mnt)" cargo test -p apsis-core --test native

# Delete with real mounts inside a snapshot: a bind mount (same filesystem) and the ext4 image
# (another filesystem, so a loop mount). Makes the folders and prints the mount commands
# (mounting needs root); `just test-nested-mount` then checks both survive a delete.
nested-dir := cargo-target-dir / 'nested-mount'
nested-snap := nested-dir / 'repo' / 'timeshift' / 'snapshots' / '2026-01-01_00-00-00'

nested-mount:
    test -d {{ext4-dir}}/mnt/lost+found || { echo "mount the ext4 image first (just ext4-image)"; exit 1; }
    mkdir -p {{nested-snap}}/localhost/bind {{nested-snap}}/localhost/loop {{nested-dir}}/outside
    printf 'x' > {{nested-snap}}/info.json
    : > {{nested-snap}}/exclude.list
    printf 'bind canary\n' > {{nested-dir}}/outside/canary
    printf 'loop canary\n' > {{ext4-dir}}/mnt/nested-mount-canary
    @echo "now: sudo mount --bind $(realpath {{nested-dir}}/outside) $(realpath {{nested-snap}}/localhost/bind)"
    @echo "and: sudo mount --bind $(realpath {{ext4-dir}}/mnt) $(realpath {{nested-snap}}/localhost/loop)"
    @echo "then: just test-nested-mount"

test-nested-mount:
    APSIS_NESTED_MOUNT="$(realpath {{nested-dir}}/repo)" cargo test -p apsis-core --test native delete_leaves_real_mounts_inside_alone -- --nocapture
    @echo "afterwards: sudo umount $(realpath {{nested-snap}}/localhost/bind) $(realpath {{nested-snap}}/localhost/loop)"

# Run the application for testing purposes
run *args:
    env RUST_BACKTRACE=full cargo run --release -p {{name}} {{args}}

# Installs files
install:
    install -Dm0755 {{ cargo-target-dir / 'release' / name }} {{bin-dst}}
    install -Dm0644 {{ 'target' / 'xdgen' / 'app.desktop' }} {{desktop-dst}}
    install -Dm0644 {{ 'target' / 'xdgen' / 'launcher.desktop' }} {{launcher-dst}}
    install -Dm0644 {{ 'target' / 'xdgen' / 'app.metainfo.xml' }} {{metainfo-dst}}
    install -Dm0644 {{ 'resources' / 'icons' / 'hicolor' / 'scalable' / 'apps' / appid + '.svg' }} {{icon-dst}}
    install -Dm0644 {{ 'resources' / 'icons' / 'hicolor' / 'symbolic' / 'apps' / appid + '-symbolic.svg' }} {{icon-symbolic-dst}}
    gzip -9nc {{ 'docs' / name + '.1' }} | install -Dm0644 /dev/stdin {{man-dst}}
    install -Dm0755 {{ cargo-target-dir / 'release' / helper }} {{helper-dst}}
    sed 's|@libexecdir@|{{libexec-path}}|g' {{ helper-res / helper-bus + '.service.in' }} | install -Dm0644 /dev/stdin {{dbus-service-dst}}
    sed 's|@libexecdir@|{{libexec-path}}|g' {{ helper-res / helper + '.service.in' }} | install -Dm0644 /dev/stdin {{systemd-unit-dst}}
    install -Dm0644 {{ helper-res / helper-bus + '.conf' }} {{dbus-policy-dst}}
    install -Dm0644 {{ helper-res / appid + '.policy' }} {{polkit-dst}}
    {{reload-system}}

# Uninstalls installed files
uninstall:
    if [ -z '{{rootdir}}' ]; then systemctl stop {{helper}}.service || true; fi
    rm -f {{helper-dst}} {{dbus-service-dst}} {{systemd-unit-dst}} {{dbus-policy-dst}} {{polkit-dst}}
    {{reload-system}}
    rm -f {{launcher-dst}} {{man-dst}}
    rm {{bin-dst}} {{desktop-dst}} {{metainfo-dst}} {{icon-dst}} {{icon-symbolic-dst}}

# .deb package (cargo-deb, [package.metadata.deb] in crates/apsis/Cargo.toml). Holds the same
# files as `install`: the templates are rendered with the same prefix, and the man page gzipped,
# into deb-assets-dir, and the maintainer scripts in resources/deb/ do the reload-system step.
# Extra args go to cargo build (CI passes --locked).
# Output: target/debian/apsis_<version>-1_amd64.deb
deb-assets-dir := 'target' / 'deb-assets'

# What a helper built with the dev-only `luks-spike` feature carries (MARKER in
# crates/apsis-helper/src/luks_spike.rs): `deb` refuses to package a helper that has it.
luks-spike-marker := 'luks-spike build (dev only)'
helper-built := cargo-target-dir / 'release' / helper

deb *args: _deb-assets
    cargo build --release --workspace {{args}}
    @if grep -qaF '{{luks-spike-marker}}' {{helper-built}}; then echo "refused: {{helper-built}} was built with the dev-only luks-spike feature"; exit 1; fi
    cargo deb -p {{name}} --no-build

_deb-assets:
    mkdir -p {{deb-assets-dir}}
    sed 's|@libexecdir@|{{libexec-path}}|g' {{ helper-res / helper-bus + '.service.in' }} > {{ deb-assets-dir / helper-bus + '.service' }}
    sed 's|@libexecdir@|{{libexec-path}}|g' {{ helper-res / helper + '.service.in' }} > {{ deb-assets-dir / helper + '.service' }}
    gzip -9nc {{ 'docs' / name + '.1' }} > {{ deb-assets-dir / name + '.1.gz' }}

# Dev only, for apsis-test and never for a release: the .deb whose helper has the `luks-spike`
# feature (a restore is let through on an encrypted or LVM system disk). Its version says so.
# Output: target/debian/apsis_<version>+luksspike-1_amd64.deb
deb-luks-spike *args: _deb-assets
    cargo build --release --workspace --features apsis-helper/luks-spike {{args}}
    grep -qaF '{{luks-spike-marker}}' {{helper-built}}
    cargo deb -p {{name}} --no-build --deb-version {{version}}+luksspike-1

# The workspace version, from [workspace.package] in the root Cargo.toml (its first `version`).
version := `sed -n '0,/^version/s/^version = "\(.*\)"$/\1/p' Cargo.toml`
deb-file := absolute_path(cargo-target-dir / 'debian' / name + '_' + version + '-1_amd64.deb')

# Testing on this machine: builds the .deb and reinstalls it with apt (asks for sudo), then
# stops the running helper so the next call starts the new one. Never `sudo just install`
# over an apt-installed Apsis: it overwrites files apt manages.
deb-install *args: (deb args)
    test -f {{deb-file}}
    sudo apt install --reinstall {{deb-file}}
    sudo systemctl stop {{helper}}.service || true
    @echo "Installed {{deb-file}}. The panel still runs the old applet: remove Apsis from the panel and add it again (or log out and in)."

# Vendors dependencies into vendor.tar (vendor/ plus a .cargo/config.toml that points cargo at it)
vendor:
    rm -rf vendor .cargo
    mkdir -p .cargo
    # --versioned-dirs: a bare vendor/xml (the `xml` crate) is where atspi-common's
    # zbus-lockstep macro looks for its XML files (../xml), and it fails to compile.
    cargo vendor --locked --versioned-dirs > .cargo/config.toml
    tar pcf vendor.tar vendor .cargo
    rm -rf vendor .cargo

# Extracts vendored dependencies
vendor-extract:
    rm -rf vendor .cargo
    tar pxf vendor.tar

# Bump cargo version, create git commit, and create tag
tag version:
    # Crates inherit the version from [workspace.package] in the root Cargo.toml.
    sed -i '0,/^version/s/^version.*/version = "{{version}}"/' Cargo.toml
    git add Cargo.toml
    cargo check --workspace
    cargo clean
    git add Cargo.lock
    git commit -m 'release: {{version}}'
    git tag -a {{version}} -m ''


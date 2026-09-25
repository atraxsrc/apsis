name := 'apsis'
appid := 'io.github.atraxsrc.Apsis'

rootdir := ''
prefix := '/usr'

metainfo := appid + '.metainfo.xml'
desktop := appid + '.desktop'

# Installation paths
base-dir := absolute_path(clean(rootdir / prefix))
cargo-target-dir := env('CARGO_TARGET_DIR', 'target')
metainfo-dst := base-dir / 'share' / 'metainfo' / metainfo
bin-dst := base-dir / 'bin' / name
desktop-dst := base-dir / 'share' / 'applications' / desktop
icons-dir := base-dir / 'share' / 'icons' / 'hicolor'
icon-dst := icons-dir / 'scalable' / 'apps' / appid + '.svg'
icon-symbolic-dst := icons-dir / 'symbolic' / 'apps' / appid + '-symbolic.svg'

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

# Compiles release profile with vendored dependencies
build-vendored *args: vendor-extract (build-release '--frozen --offline' args)

# Runs a clippy check
check *args:
    cargo clippy --workspace --all-features {{args}} -- -W clippy::pedantic

# Runs a clippy check with JSON message format
check-json: (check '--message-format=json')

# Run the application for testing purposes
run *args:
    env RUST_BACKTRACE=full cargo run --release -p {{name}} {{args}}

# Installs files
install:
    install -Dm0755 {{ cargo-target-dir / 'release' / name }} {{bin-dst}}
    install -Dm0644 {{ 'target' / 'xdgen' / 'app.desktop' }} {{desktop-dst}}
    install -Dm0644 {{ 'target' / 'xdgen' / 'app.metainfo.xml' }} {{metainfo-dst}}
    install -Dm0644 {{ 'resources' / 'icons' / 'hicolor' / 'scalable' / 'apps' / appid + '.svg' }} {{icon-dst}}
    install -Dm0644 {{ 'resources' / 'icons' / 'hicolor' / 'symbolic' / 'apps' / appid + '-symbolic.svg' }} {{icon-symbolic-dst}}
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
    rm {{bin-dst}} {{desktop-dst}} {{metainfo-dst}} {{icon-dst}} {{icon-symbolic-dst}}

# Vendor dependencies locally
vendor:
    mkdir -p .cargo
    cargo vendor --sync Cargo.toml | head -n -1 > .cargo/config.toml
    echo 'directory = "vendor"' >> .cargo/config.toml
    echo >> .cargo/config.toml
    rm -rf .cargo vendor

# Extracts vendored dependencies
vendor-extract:
    rm -rf vendor
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


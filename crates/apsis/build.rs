use std::{env, fs, path::Path};
use xdgen::{App, Context, FluentString};

fn main() {
    // `i18n/` and `resources/` live at the workspace root, two levels above this crate.
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let root = Path::new(&manifest_dir).join("../..");
    let i18n = root.join("i18n");
    let resources = root.join("resources");

    println!("cargo:rerun-if-changed={}", i18n.display());
    println!("cargo:rerun-if-changed={}", resources.display());

    let ctx = Context::new(&i18n, env::var("CARGO_PKG_NAME").unwrap()).unwrap();
    let app = App::new(FluentString("app-title"))
        .comment(FluentString("app-comment"))
        .keywords(FluentString("app-keywords"));

    // `app.desktop` is the applet's entry (panel settings only); `launcher.desktop` is the app
    // launcher's, which opens Apsis in a window.
    let desktop_entry = app
        .expand_desktop(resources.join("app.desktop"), &ctx)
        .unwrap();
    let launcher_entry = app
        .expand_desktop(resources.join("launcher.desktop"), &ctx)
        .unwrap();
    let metainfo = app
        .expand_metainfo(resources.join("app.metainfo.xml"), &ctx)
        .unwrap();

    // The justfile's `install` recipe reads these from `<workspace>/target/xdgen/`.
    let output = root.join("target/xdgen");
    fs::create_dir_all(&output).unwrap();
    fs::write(output.join("app.desktop"), desktop_entry).unwrap();
    fs::write(output.join("launcher.desktop"), launcher_entry).unwrap();
    fs::write(output.join("app.metainfo.xml"), metainfo).unwrap();
}

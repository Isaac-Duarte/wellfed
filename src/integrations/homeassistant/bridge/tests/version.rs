//! The bridge crate's version is the integration's one version: the wheel
//! takes it from here (`pyproject.toml` declares it dynamic), and the
//! integration's manifest names it twice, as its own version and as the
//! wheel it requires. A release that bumps one and not the others fails
//! here rather than in an owner's Home Assistant.

use std::path::Path;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn read(path: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(root.join(path)).unwrap()
}

#[test]
fn the_manifest_names_the_bridge_s_version() {
    let manifest: serde_json::Value =
        serde_json::from_str(&read("../custom_components/wellfed/manifest.json")).unwrap();

    assert_eq!(manifest["version"], VERSION);
    let requirements = manifest["requirements"].as_array().unwrap();
    let wanted = format!("wellfed-homeassistant=={VERSION}");
    assert_eq!(requirements, &[serde_json::Value::String(wanted)]);
}

#[test]
fn the_wheel_takes_its_version_from_the_crate() {
    let pyproject = read("pyproject.toml");
    assert!(pyproject.contains("dynamic = [\"version\"]"), "{pyproject}");
    assert!(
        !pyproject.lines().any(|line| line.starts_with("version")),
        "no version of its own: {pyproject}"
    );
}

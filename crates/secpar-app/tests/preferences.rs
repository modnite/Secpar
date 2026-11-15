//! The window's settings API without an account or a background part.
use serde_json::{json, Value};

fn call(request: Value) -> Value {
    secpar_app::call(&request.to_string())
}

#[test]
fn settings_round_trip_and_validation() {
    let folder = tempfile::tempdir().unwrap();
    let data = folder.path().join("data");
    let configured = call(json!({
        "do": "configure",
        "channel": "unused",
        "memory_store": true,
        "stream_component": folder.path().join("absent").to_string_lossy(),
        "data_folder": data.to_string_lossy(),
    }));
    assert_eq!(configured["ok"], true, "{configured}");

    let chosen = json!({
        "resolution": "1280x720",
        "fps": 120,
        "bitrate_kbps": 40000,
        "decoder": "hardware",
        "display": "fullscreen",
        "audio": "5.1-surround",
        "mute_on_focus_loss": true
    });
    let saved = call(json!({"do": "save_preferences", "preferences": chosen}));
    assert_eq!(saved["ok"], true, "{saved}");
    assert_eq!(call(json!({"do": "preferences"}))["preferences"], chosen);
    assert!(data.join("preferences.json").is_file());

    let rejected = call(json!({"do": "save_preferences", "preferences": {"fps": 999}}));
    assert_eq!(rejected["error"], "invalid_fps");
}

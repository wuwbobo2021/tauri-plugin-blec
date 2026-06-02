use std::sync::OnceLock;

use tauri::{plugin::PluginHandle, AppHandle, Wry};

static HANDLE: OnceLock<PluginHandle<Wry>> = OnceLock::new();

fn get_handle() -> &'static PluginHandle<Wry> {
    HANDLE.get().expect("plugin handle not initialized")
}

pub fn init<C: serde::de::DeserializeOwned>(
    _app: &AppHandle<Wry>,
    api: tauri::plugin::PluginApi<Wry, C>,
) -> std::result::Result<(), crate::error::Error> {
    let handle = api.register_android_plugin("com.plugin.blec", "BleClientPlugin")?;
    HANDLE.set(handle).unwrap();
    Ok(())
}

pub fn check_permissions(
    ask_if_denied: bool,
) -> std::result::Result<bool, tauri::plugin::mobile::PluginInvokeError> {
    let result: BoolResult = get_handle().run_mobile_plugin(
        "check_permissions",
        serde_json::json!({
            "askIfDenied": ask_if_denied
        }),
    )?;
    Ok(result.result)
}

#[derive(serde::Deserialize)]
struct BoolResult {
    result: bool,
}

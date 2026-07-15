use std::sync::OnceLock;

use tauri::{plugin::PluginHandle, AppHandle, Wry};

static HANDLE: OnceLock<PluginHandle<Wry>> = OnceLock::new();

// NOTE: `init_ndk_context` command exists as a workaround for
// <https://github.com/tauri-apps/tao/issues/1220>.

jni::bind_java_type! {
    AndroidContext => "android.content.Context",
}

jni::bind_java_type! {
    BleClientPlugin => "com.plugin.blec.BleClientPlugin",
    type_map = {
        AndroidContext => "android.content.Context",
    },
    native_methods_error_policy = jni::errors::LogErrorAndDefault,
    native_methods {
        fn native_init_ndk_context {
            name = "native_init_ndk_context",
            sig = (context: AndroidContext),
            fn = native_init_ndk_context_handler,
        },
    },
}

fn get_handle() -> &'static PluginHandle<Wry> {
    HANDLE.get().expect("plugin handle not initialized")
}

pub fn init<C: serde::de::DeserializeOwned>(
    _app: &AppHandle<Wry>,
    api: tauri::plugin::PluginApi<Wry, C>,
) -> std::result::Result<(), crate::error::Error> {
    let handle = api.register_android_plugin("com.plugin.blec", "BleClientPlugin")?;
    HANDLE.set(handle).unwrap();
    get_handle()
        .run_mobile_plugin::<()>("init_ndk_context", serde_json::Value::Null)
        .unwrap();
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

fn native_init_ndk_context_handler<'local>(
    env: &mut jni::Env<'local>,
    _this: BleClientPlugin<'local>,
    context: AndroidContext<'local>,
) -> Result<(), jni::errors::Error> {
    let init_needed = !do_and_forget_panic(|| {
        let _ = ndk_context::android_context();
    });
    if init_needed {
        let vm = env.get_java_vm()?.get_raw() as _;
        let ctx = env.new_global_ref(&context)?.into_raw() as _;
        if do_and_forget_panic(|| unsafe {
            ndk_context::initialize_android_context(vm, ctx);
        }) {
            tracing::info!("`ndk_context` is initialized by `tauri-plugin-blec`.")
        }
    }
    Ok(())
}

fn do_and_forget_panic(f: impl FnOnce() + std::panic::UnwindSafe) -> bool {
    let res = std::panic::catch_unwind(f);
    if res.is_err() {
        std::mem::forget(res);
        false
    } else {
        true
    }
}

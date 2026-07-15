package com.plugin.blec
 
import android.Manifest
import android.app.Activity
import android.content.Context
import android.content.Intent
import android.content.SharedPreferences
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.provider.Settings
import android.widget.Toast
import androidx.core.app.ActivityCompat
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
 
@TauriPlugin
class BleClientPlugin(activity: Activity) : Plugin(activity) {
    private val permChecker: PermissionChecker = PermissionChecker(activity, this)
    private val appContext: Context = activity.applicationContext
 
    @Command
    fun init_ndk_context(invoke: Invoke) {
        native_init_ndk_context(appContext)
        invoke.resolve()
    }
 
    private external fun native_init_ndk_context(context: Context)
 
    @InvokeArg
    class CheckPermissionsParams {
        var allowIbeacons: Boolean = false
        var askIfDenied: Boolean = false
    }
 
    @Command
    fun check_permissions(invoke: Invoke) {
        val args: CheckPermissionsParams = invoke.parseArgs(CheckPermissionsParams::class.java)
        val granted: Boolean = permChecker.checkPermissions(args.allowIbeacons, args.askIfDenied)
        val ret = JSObject()
        ret.put("result", granted)
        invoke.resolve(ret)
    }
}
 
class PermissionChecker(
    private val activity: Activity,
    private val plugin: BleClientPlugin
) {
    companion object {
        private const val PREFS_PERMISSION_FIRST_TIME_ASKING =
            "com.plugin.blec.PREFS_PERMISSION_FIRST_TIME_ASKING"
    }
 
    private fun markFirstPermissionRequest(perm: String) {
        val sharedPreference: SharedPreferences = activity.getSharedPreferences(
            PREFS_PERMISSION_FIRST_TIME_ASKING,
            Context.MODE_PRIVATE
        )
        sharedPreference.edit().putBoolean(perm, false).apply()
    }
 
    private fun firstPermissionRequest(perm: String): Boolean {
        return activity.getSharedPreferences(PREFS_PERMISSION_FIRST_TIME_ASKING, Context.MODE_PRIVATE)
            .getBoolean(perm, true)
    }
 
    fun checkPermissions(allowIbeacons: Boolean, askIfDenied: Boolean): Boolean {
        var permissions = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            arrayOf(
                Manifest.permission.BLUETOOTH_SCAN,
                Manifest.permission.BLUETOOTH_CONNECT
            )
        } else {
            arrayOf(
                Manifest.permission.BLUETOOTH_ADMIN,
                Manifest.permission.BLUETOOTH,
            )
        }
        if (allowIbeacons) {
            permissions += Manifest.permission.ACCESS_FINE_LOCATION
        }
 
        for (perm in permissions) {
            if (ActivityCompat.checkSelfPermission(activity, perm)
                != PackageManager.PERMISSION_GRANTED
            ) {
                if (firstPermissionRequest(perm)
                    || activity.shouldShowRequestPermissionRationale(perm)
                ) {
                    // this will open the permission dialog
                    markFirstPermissionRequest(perm)
                    activity.requestPermissions(permissions, 1)
                    return false
                } else {
                    if (!askIfDenied) {
                        return false
                    }
 
                    // this will open settings which asks for permission
                    val intent = Intent(
                        Settings.ACTION_APPLICATION_DETAILS_SETTINGS,
                        Uri.parse("package:" + activity.packageName)
                    )
                    activity.startActivity(intent)
                    Toast.makeText(activity, "Allow Permission: $perm", Toast.LENGTH_SHORT).show()
                    return false
                }
            }
        }
        return true
    }
}
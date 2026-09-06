//! Putting text on the clipboard: a path the user wants somewhere else.
//!
//! On desktop egui does this itself (`Context::copy_text`). On Android that
//! call is a no-op - egui-winit has no clipboard there - so the phone gets
//! one through the framework's `ClipboardManager` over JNI, the same shape
//! as the log export: framework classes only, no dex shim.

/// Copies `text` to the platform clipboard. `None` on platforms where egui's
/// own clipboard works, and the caller uses that.
pub type Copier = Box<dyn Fn(&str) -> Result<(), String>>;

/// A copier that hands the text to Android's clipboard service.
///
/// `vm` and `activity` are the pointers from `AndroidApp`, valid for the
/// process lifetime.
#[cfg(target_os = "android")]
pub fn android_copier(vm: usize, activity: usize) -> Copier {
    Box::new(move |text: &str| android::copy(vm, activity, text))
}

#[cfg(target_os = "android")]
mod android {
    use jni::objects::{JObject, JValue};
    use jni::JavaVM;

    pub fn copy(vm: usize, activity: usize, text: &str) -> Result<(), String> {
        let vm = unsafe { JavaVM::from_raw(vm as *mut jni::sys::JavaVM) }
            .map_err(|e| e.to_string())?;
        let mut env = vm.attach_current_thread().map_err(|e| e.to_string())?;
        let activity = unsafe { JObject::from_raw(activity as jni::sys::jobject) };
        // A throw leaves the exception pending for every later JNI call on
        // this thread, so it is cleared here whatever went wrong.
        let result = env
            .with_local_frame(16, |env| set_clip(env, &activity, text))
            .map_err(|e| e.to_string());
        if result.is_err() {
            let _ = env.exception_describe();
            let _ = env.exception_clear();
        }
        result
    }

    /// `ClipboardManager.setPrimaryClip(ClipData.newPlainText(label, text))`.
    fn set_clip(
        env: &mut jni::JNIEnv,
        activity: &JObject,
        text: &str,
    ) -> Result<(), jni::errors::Error> {
        let service = env.new_string("clipboard")?;
        let manager = env
            .call_method(
                activity,
                "getSystemService",
                "(Ljava/lang/String;)Ljava/lang/Object;",
                &[JValue::Object(&service)],
            )?
            .l()?;
        let label = env.new_string("gps-gui-rs")?;
        let value = env.new_string(text)?;
        let clip = env
            .call_static_method(
                "android/content/ClipData",
                "newPlainText",
                "(Ljava/lang/CharSequence;Ljava/lang/CharSequence;)Landroid/content/ClipData;",
                &[JValue::Object(&label), JValue::Object(&value)],
            )?
            .l()?;
        env.call_method(
            &manager,
            "setPrimaryClip",
            "(Landroid/content/ClipData;)V",
            &[JValue::Object(&clip)],
        )?;
        Ok(())
    }
}

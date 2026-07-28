//! Getting a recorded file off the control device.
//!
//! On desktop this is not a problem: the log is written to a path the user
//! typed and is already reachable. On Android the app writes to its private
//! data directory, which nothing but the app (and `adb`) can read - so an
//! export there means handing the bytes to the system, which is what
//! [`downloads_saver`] does.

/// Saves `contents` under `name` somewhere the user can reach, returning the
/// message to show. `None` on platforms where the log file is already in a
/// reachable place, and the caller writes it itself.
pub type Saver = Box<dyn Fn(&str, &str) -> Result<String, String>>;

/// A saver that inserts the file into the shared Downloads collection through
/// MediaStore, so it lands in the phone's Downloads folder and any file
/// manager, USB transfer or share sheet can pick it up.
///
/// MediaStore is used rather than a plain write to external storage because it
/// needs no storage permission at all on API 29+: the app owns what it
/// inserts. That is also why it is API 29+ only - before scoped storage the
/// same thing needed WRITE_EXTERNAL_STORAGE and a runtime prompt, which is not
/// worth carrying for the devices still on it.
///
/// `vm` and `activity` are the pointers from `AndroidApp`, valid for the
/// process lifetime. All of this is framework classes called over JNI - no
/// Java subclass and so no dex shim, unlike the BLE and location bridges.
#[cfg(target_os = "android")]
pub fn downloads_saver(vm: usize, activity: usize) -> Saver {
    Box::new(move |name: &str, contents: &str| {
        android::save_to_downloads(vm, activity, name, contents)
            .map(|()| format!("Saved to Downloads/{name}"))
            .map_err(|e| format!("Export failed: {e}"))
    })
}

#[cfg(target_os = "android")]
mod android {
    use jni::objects::{JObject, JValue};
    use jni::JavaVM;

    type AnyError = Box<dyn std::error::Error>;

    /// Scoped storage, and with it `MediaStore.Downloads`, arrived in API 29.
    const SCOPED_STORAGE_SDK: i32 = 29;

    pub fn save_to_downloads(
        vm: usize,
        activity: usize,
        name: &str,
        contents: &str,
    ) -> Result<(), AnyError> {
        let vm = unsafe { JavaVM::from_raw(vm as *mut jni::sys::JavaVM) }?;
        let mut env = vm.attach_current_thread()?;
        let activity = unsafe { JObject::from_raw(activity as jni::sys::jobject) };

        // A throw leaves the exception pending, and every later JNI call on
        // this thread would then fail with it. Clear it here so a refused
        // export is one error message rather than a poisoned thread.
        let result =
            env.with_local_frame(16, |env| insert(env, &activity, name, contents.as_bytes()));
        if result.is_err() {
            let _ = env.exception_describe();
            let _ = env.exception_clear();
        }
        result
    }

    fn insert(
        env: &mut jni::JNIEnv,
        activity: &JObject,
        name: &str,
        bytes: &[u8],
    ) -> Result<(), AnyError> {
        let sdk = env
            .get_static_field("android/os/Build$VERSION", "SDK_INT", "I")?
            .i()?;
        if sdk < SCOPED_STORAGE_SDK {
            return Err("this Android version has no Downloads collection".into());
        }

        let resolver = env
            .call_method(
                activity,
                "getContentResolver",
                "()Landroid/content/ContentResolver;",
                &[],
            )?
            .l()?;

        // ContentValues describing the file. The column names are the string
        // constants MediaStore itself uses, spelled out rather than read back
        // off the class - they are part of the published contract and have not
        // moved since API 1.
        let values = env.new_object("android/content/ContentValues", "()V", &[])?;
        put(env, &values, "_display_name", name)?;
        put(env, &values, "mime_type", "text/csv")?;
        // Relative to the shared storage root, so the file shows up where a
        // browser download would.
        put(env, &values, "relative_path", "Download")?;

        let collection = env
            .get_static_field(
                "android/provider/MediaStore$Downloads",
                "EXTERNAL_CONTENT_URI",
                "Landroid/net/Uri;",
            )?
            .l()?;
        let uri = env
            .call_method(
                &resolver,
                "insert",
                "(Landroid/net/Uri;Landroid/content/ContentValues;)Landroid/net/Uri;",
                &[JValue::Object(&collection), JValue::Object(&values)],
            )?
            .l()?;
        if uri.is_null() {
            return Err("MediaStore would not create the file".into());
        }

        let stream = env
            .call_method(
                &resolver,
                "openOutputStream",
                "(Landroid/net/Uri;)Ljava/io/OutputStream;",
                &[JValue::Object(&uri)],
            )?
            .l()?;
        if stream.is_null() {
            return Err("MediaStore would not open the file".into());
        }
        let array = env.byte_array_from_slice(bytes)?;
        env.call_method(&stream, "write", "([B)V", &[JValue::Object(&array)])?;
        // Closing is what flushes it; a stream left to the finalizer can
        // publish a truncated file.
        env.call_method(&stream, "close", "()V", &[])?;
        Ok(())
    }

    /// `values.put(key, value)` for the String overload.
    fn put(
        env: &mut jni::JNIEnv,
        values: &JObject,
        key: &str,
        value: &str,
    ) -> Result<(), AnyError> {
        let key = env.new_string(key)?;
        let value = env.new_string(value)?;
        env.call_method(
            values,
            "put",
            "(Ljava/lang/String;Ljava/lang/String;)V",
            &[JValue::Object(&key), JValue::Object(&value)],
        )?;
        Ok(())
    }
}

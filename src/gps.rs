//! GPS position source.
//!
//! The rest of the app only depends on an optional [`GpsHandle`]: something
//! produces fixes on a background thread and sends them over a channel, the
//! UI drains that channel each frame, and a flag says whether the receiver
//! should be running at all. Android reads the phone's GNSS via
//! LocationManager. Desktop has no built-in source yet, so the UI shows a
//! manual position entry bar instead (see `app::MyApp`); a real source can
//! slot in the same way later.

use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Receiver;
use std::sync::Arc;

#[cfg(target_os = "android")]
use std::sync::atomic::Ordering;
#[cfg(target_os = "android")]
use std::sync::mpsc::channel;
#[cfg(target_os = "android")]
use std::thread;
#[cfg(target_os = "android")]
use std::time::{Duration, Instant};

/// A single GPS fix in decimal degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GpsFix {
    pub lat: f64,
    pub lon: f64,
    /// Course over ground: degrees clockwise from true north. Only present when
    /// moving (GPS cannot derive a bearing while stationary).
    pub bearing: Option<f32>,
    /// Ground speed in meters per second. Optional for the same reason the
    /// bearing is: a provider that cannot measure it reports none at all,
    /// which must not read as a stationary zero.
    pub speed: Option<f32>,
}

/// The app's side of a GPS source: the fixes, and whether the receiver
/// should be running.
///
/// The flag is what `[phone] location` turns: with it clear the source
/// asks the platform for nothing - no permission dialog, no receiver - so
/// a phone that is only ever the far end of a node's link never powers its
/// own GNSS for a position nobody reads.
pub struct GpsHandle {
    pub fixes: Receiver<GpsFix>,
    pub wanted: Arc<AtomicBool>,
}

/// How often the source thread looks at the flag while idle, and between
/// fixes while running.
#[cfg(target_os = "android")]
const WANTED_POLL: Duration = Duration::from_millis(500);

/// Spawn a GPS source backed by the phone's Android LocationManager.
///
/// Starts idle. Once the handle's flag is raised it requests the
/// fine-location permission if needed, seeds the map with the freshest
/// last-known fix, then registers for active location updates and emits
/// each fresh fix on change over the channel; when the flag is lowered it
/// removes the updates again and goes back to waiting.
///
/// Active updates matter: `getLastKnownLocation` is passive and never wakes the
/// GNSS hardware, so on its own it returns a stale fix that never changes. The
/// live updates come through a small `LocationListener` dex shim
/// (android/LocationBridge.java), which is what actually powers up the GNSS.
///
/// `vm` and `activity` are the raw `JavaVM` and Activity pointers from
/// `AndroidApp` (passed as `usize` so they can cross the thread boundary). The
/// Activity is required: `requestPermissions` is an Activity method, and
/// `ndk_context`'s context is the Application, which does not have it.
#[cfg(target_os = "android")]
pub fn spawn_android_location(ctx: egui::Context, vm: usize, activity: usize) -> GpsHandle {
    let (tx, rx) = channel();
    let wanted = Arc::new(AtomicBool::new(false));
    let flag = wanted.clone();

    thread::spawn(move || {
        if let Err(err) = android_location_loop(&tx, &ctx, &flag, vm, activity) {
            log::error!("android location source stopped: {err}");
        }
    });

    GpsHandle { fixes: rx, wanted }
}

#[cfg(target_os = "android")]
fn android_location_loop(
    tx: &std::sync::mpsc::Sender<GpsFix>,
    ctx: &egui::Context,
    wanted: &AtomicBool,
    vm: usize,
    activity: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    use jni::objects::{JObject, JValue};
    use jni::JavaVM;

    // Pointers from AndroidApp, valid for the process lifetime.
    let vm = unsafe { JavaVM::from_raw(vm as *mut jni::sys::JavaVM) }?;
    let activity = unsafe { JObject::from_raw(activity as jni::sys::jobject) };
    let mut env = vm.attach_current_thread()?;

    // The callback channel and the shim class are made once and kept across
    // every start and stop: the shim binds its native method to the channel
    // it is given, and a class is loaded from the dex once per process.
    let (loc_tx, loc_rx) = channel();
    LOC_TX
        .set(std::sync::Mutex::new(loc_tx))
        .map_err(|_| "android location started twice")?;
    let mut class: Option<jni::objects::GlobalRef> = None;
    let mut last: Option<GpsFix> = None;

    loop {
        // Nothing is asked of the platform until a position is wanted.
        while !wanted.load(Ordering::Relaxed) {
            thread::sleep(WANTED_POLL);
        }

        // Ensure the fine-location permission is granted (poll after
        // prompting; the native activity has no easy
        // onRequestPermissionsResult callback).
        //
        // Both permissions go in the one request: since Android 12 a request
        // for ACCESS_FINE_LOCATION that omits ACCESS_COARSE_LOCATION is
        // ignored outright - no dialog, no denial, only a logcat line - and
        // the poll below would then wait forever for an answer that was never
        // asked for.
        const FINE: &str = "android.permission.ACCESS_FINE_LOCATION";
        const COARSE: &str = "android.permission.ACCESS_COARSE_LOCATION";
        let mut last_request: Option<Instant> = None;
        let mut warned_coarse = false;
        let mut granted = false;
        while wanted.load(Ordering::Relaxed) {
            if check_permission(&mut env, &activity, FINE)? {
                granted = true;
                break;
            }
            // "Approximate" on the Android 12+ dialog grants coarse and
            // denies fine. That is a deliberate answer, so stop asking and
            // wait for a change from Settings instead of putting the dialog
            // up on a timer.
            if check_permission(&mut env, &activity, COARSE)? {
                if !warned_coarse {
                    warned_coarse = true;
                    log::warn!(
                        "only approximate location granted; the map needs precise \
                         location, grant it in Settings"
                    );
                }
            } else if last_request.map_or(true, |t| t.elapsed() > Duration::from_secs(20)) {
                // Re-request rather than ask once: the BLE worker may be
                // putting its own dialog up at startup, which drops ours.
                last_request = Some(Instant::now());
                request_permissions(&mut env, &activity, &[FINE, COARSE])?;
            }
            thread::sleep(WANTED_POLL);
        }
        if !granted {
            // The switch went off while the dialog was up.
            continue;
        }

        // LocationManager lm = activity.getSystemService("location");
        let service = env.new_string("location")?;
        let location_manager = env
            .call_method(
                &activity,
                "getSystemService",
                "(Ljava/lang/String;)Ljava/lang/Object;",
                &[JValue::Object(&service)],
            )?
            .l()?;

        // Seed the map with the freshest last-known fix so it is not empty
        // before the first live update arrives (a cold GPS fix can take tens
        // of seconds).
        if let Some(fix) =
            last_known_fix(&mut env, &location_manager, &["gps", "fused", "network", "passive"])?
        {
            last = Some(fix);
            if tx.send(fix).is_err() {
                return Ok(()); // UI has gone away.
            }
            ctx.request_repaint();
        }

        // This thread never returns to Java, so the references made per
        // start are dropped by hand rather than left to accumulate across
        // every off-and-on of the switch.
        env.delete_local_ref(location_manager)?;
        env.delete_local_ref(service)?;

        // Register for active updates through the dex shim. This is what
        // powers up the GNSS hardware; the last-known sweep above is passive
        // and never refreshes on its own. Fixes land on `loc_rx` via
        // `native_on_location`.
        if class.is_none() {
            class = Some(load_location_bridge(&mut env, &activity)?);
        }
        let class = class.as_ref().expect("loaded above");

        // LocationBridge.start(activity, minTimeMs = 1000, minDistanceM = 0)
        let started = env
            .call_static_method(
                class,
                "start",
                "(Landroid/app/Activity;JF)Z",
                &[JValue::Object(&activity), JValue::Long(1000), JValue::Float(0.0)],
            )?
            .z()?;
        if !started {
            log::warn!("no location provider available; showing last-known fix only");
        }

        // Pump until the switch goes off. The Sender in LOC_TX lives forever,
        // so a disconnect here is the UI having gone away.
        loop {
            match loc_rx.recv_timeout(WANTED_POLL) {
                Ok(fix) => {
                    if last != Some(fix) {
                        last = Some(fix);
                        if tx.send(fix).is_err() {
                            let _ = env.call_static_method(class, "stop", "()V", &[]);
                            return Ok(()); // UI has gone away.
                        }
                        ctx.request_repaint();
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
            }
            if !wanted.load(Ordering::Relaxed) {
                // Off: the receiver goes down with the updates, which is the
                // whole saving.
                let _ = env.call_static_method(class, "stop", "()V", &[]);
                break;
            }
        }
    }
}

/// Freshest last-known location across the given providers, or `None` if none
/// have one cached. Passive: this never wakes the GNSS hardware, so it only
/// seeds an initial position; live movement comes from the LocationListener.
#[cfg(target_os = "android")]
fn last_known_fix(
    env: &mut jni::JNIEnv,
    location_manager: &jni::objects::JObject,
    providers: &[&str],
) -> Result<Option<GpsFix>, jni::errors::Error> {
    use jni::objects::JValue;

    // Scope JNI local references: the caller runs on a long-lived thread that
    // never returns to Java, so per-call refs must not accumulate.
    env.with_local_frame(16, |env| -> Result<Option<GpsFix>, jni::errors::Error> {
        // Pick the most recent last-known location across providers.
        let mut best: Option<(GpsFix, i64)> = None;

        for provider in providers {
            let name = env.new_string(provider)?;
            let location = match env.call_method(
                location_manager,
                "getLastKnownLocation",
                "(Ljava/lang/String;)Landroid/location/Location;",
                &[JValue::Object(&name)],
            ) {
                Ok(value) => value.l()?,
                Err(_) => continue, // provider not present on this device
            };
            if location.is_null() {
                continue;
            }

            let lat = env.call_method(&location, "getLatitude", "()D", &[])?.d()?;
            let lon = env.call_method(&location, "getLongitude", "()D", &[])?.d()?;
            let time = env.call_method(&location, "getTime", "()J", &[])?.j()?;

            // Course over ground and ground speed, only valid while moving.
            let bearing = if env.call_method(&location, "hasBearing", "()Z", &[])?.z()? {
                Some(env.call_method(&location, "getBearing", "()F", &[])?.f()?)
            } else {
                None
            };
            let speed = if env.call_method(&location, "hasSpeed", "()Z", &[])?.z()? {
                Some(env.call_method(&location, "getSpeed", "()F", &[])?.f()?)
            } else {
                None
            };

            if best.map_or(true, |(_, t)| time > t) {
                best = Some((GpsFix { lat, lon, bearing, speed }, time));
            }
        }

        Ok(best.map(|(fix, _)| fix))
    })
}

/// Internal channel from the Java `LocationListener` callback (main thread) to
/// the location worker. Set once before `LocationBridge.start`, so the callback
/// can never fire before it exists.
#[cfg(target_os = "android")]
static LOC_TX: std::sync::OnceLock<std::sync::Mutex<std::sync::mpsc::Sender<GpsFix>>> =
    std::sync::OnceLock::new();

/// `LocationBridge.nativeOnLocation`: one live fix. Runs on the main thread and
/// only pushes onto the channel the worker drains, so no locking is needed
/// beyond the channel's.
#[cfg(target_os = "android")]
extern "system" fn native_on_location(
    _env: jni::JNIEnv,
    _class: jni::objects::JClass,
    lat: jni::sys::jdouble,
    lon: jni::sys::jdouble,
    bearing: jni::sys::jfloat,
    has_bearing: jni::sys::jboolean,
    speed: jni::sys::jfloat,
    has_speed: jni::sys::jboolean,
) {
    let fix = GpsFix {
        lat,
        lon,
        bearing: (has_bearing != 0).then_some(bearing),
        speed: (has_speed != 0).then_some(speed),
    };
    if let Some(tx) = LOC_TX.get() {
        if let Ok(tx) = tx.lock() {
            let _ = tx.send(fix);
        }
    }
}

/// Write the embedded dex holding `rs.gps.gui.LocationBridge` into the code
/// cache dir, load it with DexClassLoader (FindClass cannot see dex classes),
/// and bind `nativeOnLocation`. Mirrors the BLE bridge loader.
#[cfg(target_os = "android")]
fn load_location_bridge(
    env: &mut jni::JNIEnv,
    activity: &jni::objects::JObject,
) -> Result<jni::objects::GlobalRef, Box<dyn std::error::Error>> {
    use jni::objects::{JClass, JObject, JString, JValue};
    use jni::NativeMethod;

    const LOCATION_DEX: &[u8] = include_bytes!("../assets/location-bridge.dex");
    const LOCATION_CLASS: &str = "rs.gps.gui.LocationBridge";

    let dir = env
        .call_method(activity, "getCodeCacheDir", "()Ljava/io/File;", &[])?
        .l()?;
    let dir: JString = env
        .call_method(&dir, "getAbsolutePath", "()Ljava/lang/String;", &[])?
        .l()?
        .into();
    let dir: String = env.get_string(&dir)?.into();
    let dex_path = format!("{dir}/location-bridge.dex");
    std::fs::write(&dex_path, LOCATION_DEX)?;

    // new DexClassLoader(dexPath, codeCacheDir, null, activity.getClassLoader())
    let parent = env
        .call_method(activity, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?
        .l()?;
    let j_dex_path = env.new_string(&dex_path)?;
    let j_opt_dir = env.new_string(&dir)?;
    let loader = env.new_object(
        "dalvik/system/DexClassLoader",
        "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/ClassLoader;)V",
        &[
            JValue::Object(&j_dex_path),
            JValue::Object(&j_opt_dir),
            JValue::Object(&JObject::null()),
            JValue::Object(&parent),
        ],
    )?;

    let j_class_name = env.new_string(LOCATION_CLASS)?;
    let class_obj = env
        .call_method(
            &loader,
            "loadClass",
            "(Ljava/lang/String;)Ljava/lang/Class;",
            &[JValue::Object(&j_class_name)],
        )?
        .l()?;
    // Keep the class as a global ref: FindClass cannot see dex-loaded classes,
    // and jni's Desc machinery accepts a &GlobalRef directly.
    let class = env.new_global_ref(&JClass::from(class_obj))?;

    env.register_native_methods(
        &class,
        &[NativeMethod {
            name: "nativeOnLocation".into(),
            sig: "(DDFZFZ)V".into(),
            fn_ptr: native_on_location as *mut _,
        }],
    )?;

    Ok(class)
}

/// `Context.checkSelfPermission(name) == PERMISSION_GRANTED (0)`.
/// Also used by the BLE worker for the Bluetooth runtime permissions.
#[cfg(target_os = "android")]
pub(crate) fn check_permission(
    env: &mut jni::JNIEnv,
    activity: &jni::objects::JObject,
    permission: &str,
) -> Result<bool, jni::errors::Error> {
    use jni::objects::JValue;
    // Scoped frame: this is polled in a loop, so per-call refs must not leak.
    env.with_local_frame(8, |env| -> Result<bool, jni::errors::Error> {
        let name = env.new_string(permission)?;
        let granted = env
            .call_method(
                activity,
                "checkSelfPermission",
                "(Ljava/lang/String;)I",
                &[JValue::Object(&name)],
            )?
            .i()?
            == 0;
        Ok(granted)
    })
}

/// `Activity.requestPermissions(names, requestCode)`.
///
/// Takes the whole set at once because Android requires related permissions to
/// be asked for together (fine and coarse location since Android 12), and
/// because one dialog for the set beats one per permission.
///
/// Best-effort: if the call throws (e.g. some devices insist it run on the UI
/// thread), the pending exception is cleared and logged rather than left to
/// crash the process. The caller polls `checkSelfPermission` regardless, so the
/// user can also grant the permission from Settings or via `adb`.
#[cfg(target_os = "android")]
fn request_permissions(
    env: &mut jni::JNIEnv,
    activity: &jni::objects::JObject,
    perms: &[&str],
) -> Result<(), jni::errors::Error> {
    use jni::objects::{JObject, JValue};
    // Scoped frame: this can run every 20s, so per-call refs must not leak.
    let result = env.with_local_frame(
        8 + perms.len() as i32,
        |env| -> Result<(), jni::errors::Error> {
            let array =
                env.new_object_array(perms.len() as i32, "java/lang/String", JObject::null())?;
            for (i, p) in perms.iter().enumerate() {
                let name = env.new_string(p)?;
                env.set_object_array_element(&array, i as i32, &name)?;
            }
            env.call_method(
                activity,
                "requestPermissions",
                "([Ljava/lang/String;I)V",
                &[JValue::Object(&array), JValue::Int(1)],
            )?;
            Ok(())
        },
    );

    if env.exception_check()? {
        let _ = env.exception_describe();
        env.exception_clear()?;
    }
    if let Err(err) = result {
        log::warn!("requestPermissions failed ({err}); grant location manually in Settings");
    }

    Ok(())
}

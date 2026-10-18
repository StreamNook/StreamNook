//! The slice of Tauri's API the StreamNook core uses, without a webview.
//!
//! On iOS the Swift app is the shell: it starts the core through UniFFI,
//! calls commands by name with JSON arguments, and receives events through a
//! [`HostSink`]. The crate root aliases this crate as `tauri` there, so the
//! core's commands, `use tauri::Emitter` lines and builder chain compile as
//! written. Every app handle shares one process-wide state: there is only
//! ever one app.
//!
//! What has no meaning without windows answers "nothing": window lookups
//! return `None`, window builders fail with [`Error::Unsupported`], plugins
//! are not registered. Requests only the shell can serve (`exit`, `restart`)
//! go to [`HostSink::host_request`].

use serde::Serialize;
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::fmt;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

pub use sn_tauri_headless_macros::{command, generate_context, generate_handler, mobile_entry_point};
pub use url::Url;

pub use crate::window::{
    LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize, Position, Size, Window,
    WindowEvent,
};
pub use crate::webview::{Webview, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

/// The exit code Tauri uses to ask for a relaunch.
pub const RESTART_EXIT_CODE: i32 = i32::MAX;

// ---------------------------------------------------------------------------
// Runtime marker

/// Stands in for Tauri's runtime trait. There is one runtime, [`Wry`].
pub trait Runtime: Send + Sync + 'static {}

/// The only runtime. Named `Wry` so `crate::rt::Rt = tauri::Wry` holds on iOS.
#[derive(Debug, Clone, Copy, Default)]
pub struct Wry;

impl Runtime for Wry {}

pub type EventLoopMessage = ();

// ---------------------------------------------------------------------------
// Errors

#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The operation needs a window, webview or plugin the iOS shell does not have.
    Unsupported(&'static str),
    /// The host has not installed its directories yet.
    HostNotInstalled,
    Json(serde_json::Error),
    Io(std::io::Error),
    /// A spawned task panicked or was cancelled.
    Join(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unsupported(what) => write!(f, "{what} is not available on iOS"),
            Error::HostNotInstalled => write!(f, "the iOS host has not started the core"),
            Error::Json(e) => write!(f, "{e}"),
            Error::Io(e) => write!(f, "{e}"),
            Error::Join(e) => write!(f, "task failed: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Json(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

// ---------------------------------------------------------------------------
// Host

/// Where events and shell requests go. The UniFFI layer adapts the Swift
/// implementation to this trait.
pub trait HostSink: Send + Sync {
    /// An event the core emitted. `target` is the label from `emit_to`, or
    /// `None` for a broadcast.
    fn event(&self, name: &str, payload_json: String, target: Option<String>);
    /// Something only the shell can do (`exit`, `restart`, ...).
    fn host_request(&self, kind: &str, payload_json: String);
}

/// The app's sandbox directories, chosen by the shell.
#[derive(Debug, Clone)]
pub struct HostDirs {
    pub data: PathBuf,
    pub cache: PathBuf,
    pub logs: PathBuf,
}

struct Host {
    sink: Arc<dyn HostSink>,
    dirs: HostDirs,
    version: String,
}

static HOST: OnceLock<Host> = OnceLock::new();

fn host() -> Option<&'static Host> {
    HOST.get()
}

/// Installs the shell's sink and directories. Returns false when a host is
/// already installed (the first one stays).
pub fn host_install(sink: Arc<dyn HostSink>, dirs: HostDirs, version: String) -> bool {
    HOST.set(Host { sink, dirs, version }).is_ok()
}

/// The shell's own version string, as passed to [`host_install`].
pub fn host_version() -> Option<&'static str> {
    host().map(|h| h.version.as_str())
}

/// Sends a request only the shell can serve. Dropped when no host is installed.
pub fn host_request<S: Serialize>(kind: &str, payload: &S) {
    match host() {
        Some(h) => {
            let json = serde_json::to_string(payload).unwrap_or_else(|_| "null".to_string());
            h.sink.host_request(kind, json);
        }
        None => log::warn!("[Headless] host request {kind} dropped: no host installed"),
    }
}

/// Runs a command by name with a JSON arguments object. Resolves to the
/// command's JSON result, or to a JSON error value (the command's own error,
/// or a string when the command is unknown or the arguments do not parse).
pub fn host_dispatch(
    cmd: &str,
    args_json: &str,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = std::result::Result<String, String>> + Send>>
{
    fn error(message: String) -> String {
        serde_json::Value::String(message).to_string()
    }
    let prepared = (|| {
        let handler = HANDLER
            .get()
            .ok_or_else(|| error("the core has not started".to_string()))?;
        let args: serde_json::Value = if args_json.trim().is_empty() {
            serde_json::Value::Object(Default::default())
        } else {
            serde_json::from_str(args_json)
                .map_err(|e| error(format!("arguments for {cmd} are not JSON: {e}")))?
        };
        (handler.0)(cmd, AppHandle::new(), args).ok_or_else(|| error(format!("unknown command {cmd}")))
    })();
    Box::pin(async move {
        match prepared?.await {
            Ok(value) => Ok(value.to_string()),
            Err(value) => Err(value.to_string()),
        }
    })
}

/// Delivers a run event to the callback `App::run` was given (for example
/// `RunEvent::Exit` when the shell is terminating).
pub fn host_run_event(event: RunEvent) {
    let callback = RUN_CALLBACK.lock().ok().and_then(|mut slot| slot.take());
    if let Some(mut callback) = callback {
        callback(&AppHandle::new(), event);
        if let Ok(mut slot) = RUN_CALLBACK.lock() {
            if slot.is_none() {
                *slot = Some(callback);
            }
        }
    }
}

static HANDLER: OnceLock<ipc::Handler> = OnceLock::new();
static CONTEXT: OnceLock<Context> = OnceLock::new();
type RunCallback = Box<dyn FnMut(&AppHandle, RunEvent) + Send>;
static RUN_CALLBACK: Mutex<Option<RunCallback>> = Mutex::new(None);

fn context() -> &'static Context {
    CONTEXT.get_or_init(Context::default)
}

// ---------------------------------------------------------------------------
// Context and config

/// The parts of Tauri's config the core reads.
#[derive(Debug, Clone, Default)]
pub struct Config {
    pub product_name: Option<String>,
    pub version: Option<String>,
    pub identifier: String,
    pub build: BuildConfig,
    pub app: AppConfig,
}

#[derive(Debug, Clone, Default)]
pub struct BuildConfig {
    pub dev_url: Option<Url>,
}

#[derive(Debug, Clone, Default)]
pub struct AppConfig {
    pub windows: Vec<WindowConfig>,
}

#[derive(Debug, Clone, Default)]
pub struct WindowConfig {
    pub label: String,
    pub transparent: bool,
}

#[derive(Debug, Clone)]
pub struct PackageInfo {
    pub name: String,
    pub version: semver::Version,
    pub authors: &'static str,
    pub description: &'static str,
    pub crate_name: &'static str,
}

impl PackageInfo {
    pub fn app_name(&self) -> &str {
        &self.name
    }
}

#[derive(Debug, Clone)]
pub struct Context {
    config: Config,
    package_info: PackageInfo,
}

impl Default for Context {
    fn default() -> Self {
        Context::__new("app", "0.0.0")
    }
}

impl Context {
    #[doc(hidden)]
    pub fn __new(name: &'static str, version: &'static str) -> Self {
        let version = semver::Version::parse(version).unwrap_or(semver::Version::new(0, 0, 0));
        Context {
            config: Config {
                product_name: Some(name.to_string()),
                version: Some(version.to_string()),
                identifier: String::new(),
                build: BuildConfig::default(),
                app: AppConfig::default(),
            },
            package_info: PackageInfo {
                name: name.to_string(),
                version,
                authors: "",
                description: "",
                crate_name: name,
            },
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn config_mut(&mut self) -> &mut Config {
        &mut self.config
    }

    pub fn package_info(&self) -> &PackageInfo {
        &self.package_info
    }
}

// ---------------------------------------------------------------------------
// Managed state

type StateMap = HashMap<TypeId, &'static (dyn Any + Send + Sync)>;

fn states() -> &'static RwLock<StateMap> {
    static STATES: OnceLock<RwLock<StateMap>> = OnceLock::new();
    STATES.get_or_init(Default::default)
}

fn manage_state<T: Send + Sync + 'static>(value: T) -> bool {
    let mut map = match states().write() {
        Ok(map) => map,
        Err(poisoned) => poisoned.into_inner(),
    };
    if map.contains_key(&TypeId::of::<T>()) {
        return false;
    }
    // Managed state lives for the whole process, as it does in Tauri.
    let leaked: &'static T = Box::leak(Box::new(value));
    map.insert(TypeId::of::<T>(), leaked);
    true
}

fn get_state<T: Send + Sync + 'static>() -> Option<State<'static, T>> {
    let map = match states().read() {
        Ok(map) => map,
        Err(poisoned) => poisoned.into_inner(),
    };
    map.get(&TypeId::of::<T>())
        .and_then(|value| value.downcast_ref::<T>())
        .map(State)
}

/// A reference to managed state, as Tauri hands it to commands.
pub struct State<'r, T: Send + Sync + 'static>(&'r T);

impl<'r, T: Send + Sync + 'static> State<'r, T> {
    pub fn inner(&self) -> &'r T {
        self.0
    }
}

impl<T: Send + Sync + 'static> std::ops::Deref for State<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.0
    }
}

impl<T: Send + Sync + 'static> Clone for State<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: Send + Sync + 'static> Copy for State<'_, T> {}

impl<T: Send + Sync + 'static + fmt::Debug> fmt::Debug for State<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("State").field(self.0).finish()
    }
}

// ---------------------------------------------------------------------------
// App, AppHandle, Builder

/// A handle to the app. Every handle refers to the same process-wide app.
pub struct AppHandle<R: Runtime = Wry> {
    path: path::PathResolver<R>,
}

impl<R: Runtime> AppHandle<R> {
    pub(crate) fn new() -> Self {
        AppHandle {
            path: path::PathResolver(PhantomData),
        }
    }

    /// Asks the shell to end the app. iOS apps do not quit themselves, so the
    /// shell decides what this means.
    pub fn exit(&self, code: i32) {
        host_request("exit", &serde_json::json!({ "code": code }));
    }

    /// Asks the shell to relaunch the app, then ends the process.
    pub fn restart(&self) -> ! {
        host_request("restart", &serde_json::Value::Null);
        std::process::exit(0)
    }

    pub fn request_restart(&self) {
        host_request("restart", &serde_json::Value::Null);
    }

    /// There is no UI thread to hop to; the task runs on a runtime worker.
    pub fn run_on_main_thread<F: FnOnce() + Send + 'static>(&self, f: F) -> Result<()> {
        async_runtime::spawn_blocking(f);
        Ok(())
    }

    pub fn cleanup_before_exit(&self) {}

    pub fn default_window_icon(&self) -> Option<&image::Image<'_>> {
        None
    }
}

impl<R: Runtime> Clone for AppHandle<R> {
    fn clone(&self) -> Self {
        AppHandle::new()
    }
}

impl<R: Runtime> fmt::Debug for AppHandle<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AppHandle")
    }
}

type SetupHook<R> =
    Box<dyn FnOnce(&mut App<R>) -> std::result::Result<(), Box<dyn std::error::Error>> + Send>;

/// The built app. [`App::run`] runs the setup hook, sends `RunEvent::Ready`
/// and returns: the shell owns the main loop.
pub struct App<R: Runtime = Wry> {
    handle: AppHandle<R>,
    setup: Option<SetupHook<R>>,
    handler: Option<ipc::Handler>,
}

impl<R: Runtime> App<R> {
    pub fn handle(&self) -> &AppHandle<R> {
        &self.handle
    }

    pub fn default_window_icon(&self) -> Option<&image::Image<'_>> {
        None
    }

    pub fn run<F>(mut self, callback: F)
    where
        F: FnMut(&AppHandle<R>, RunEvent) + Send + 'static,
    {
        let _runtime = async_runtime::runtime().enter();
        if let Some(handler) = self.handler.take() {
            if HANDLER.set(handler).is_err() {
                log::warn!("[Headless] a command handler was already installed");
            }
        }
        if let Some(setup) = self.setup.take() {
            if let Err(e) = setup(&mut self) {
                panic!("Failed to setup app: {e}");
            }
        }
        let mut callback = callback;
        callback(&self.handle, RunEvent::Ready);
        let stored: RunCallback = Box::new(move |_handle: &AppHandle, event| {
            callback(&AppHandle::new(), event)
        });
        if let Ok(mut slot) = RUN_CALLBACK.lock() {
            *slot = Some(stored);
        }
    }
}

/// Builds the app. Plugins are accepted and ignored; managed state is stored
/// as soon as it is handed over.
pub struct Builder<R: Runtime = Wry> {
    setup: Option<SetupHook<R>>,
    handler: Option<ipc::Handler>,
    _runtime: PhantomData<fn() -> R>,
}

impl<R: Runtime> Default for Builder<R> {
    fn default() -> Self {
        Builder {
            setup: None,
            handler: None,
            _runtime: PhantomData,
        }
    }
}

impl<R: Runtime> Builder<R> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn plugin<P>(self, _plugin: P) -> Self {
        self
    }

    pub fn manage<T: Send + Sync + 'static>(self, state: T) -> Self {
        manage_state(state);
        self
    }

    pub fn setup<F>(mut self, setup: F) -> Self
    where
        F: FnOnce(&mut App<R>) -> std::result::Result<(), Box<dyn std::error::Error>>
            + Send
            + 'static,
    {
        self.setup = Some(Box::new(setup));
        self
    }

    pub fn invoke_handler(mut self, handler: ipc::Handler) -> Self {
        self.handler = Some(handler);
        self
    }

    /// No windows exist, so the handler is never called.
    pub fn on_window_event<F>(self, _handler: F) -> Self
    where
        F: Fn(&Window<R>, &WindowEvent) + Send + Sync + 'static,
    {
        self
    }

    pub fn build(self, mut context: Context) -> Result<App<R>> {
        // The app's version is the shell's own line (the version it passed to
        // `host_install`), not the crate version the context was built with.
        if let Some(version) = host_version().and_then(|v| semver::Version::parse(v).ok()) {
            context.config.version = Some(version.to_string());
            context.package_info.version = version;
        }
        let _ = CONTEXT.set(context);
        Ok(App {
            handle: AppHandle::new(),
            setup: self.setup,
            handler: self.handler,
        })
    }
}

/// Events the app's run callback receives.
#[derive(Debug)]
#[non_exhaustive]
pub enum RunEvent {
    Exit,
    ExitRequested {
        code: Option<i32>,
        api: ExitRequestApi,
    },
    WindowEvent {
        label: String,
        event: WindowEvent,
    },
    Ready,
    Resumed,
    MainEventsCleared,
}

#[derive(Debug, Clone, Default)]
pub struct ExitRequestApi(());

impl ExitRequestApi {
    pub fn prevent_exit(&self) {}
}

// ---------------------------------------------------------------------------
// Manager, Emitter, Listener

#[doc(hidden)]
pub trait ManagerBase<R: Runtime> {
    fn __handle(&self) -> &AppHandle<R>;
}

pub trait Manager<R: Runtime>: ManagerBase<R> {
    fn app_handle(&self) -> &AppHandle<R> {
        self.__handle()
    }

    fn config(&self) -> &Config {
        &context().config
    }

    fn package_info(&self) -> &PackageInfo {
        &context().package_info
    }

    fn manage<T: Send + Sync + 'static>(&self, state: T) -> bool {
        manage_state(state)
    }

    fn state<T: Send + Sync + 'static>(&self) -> State<'_, T> {
        get_state::<T>().unwrap_or_else(|| {
            panic!(
                "state() called before manage() for {}",
                std::any::type_name::<T>()
            )
        })
    }

    fn try_state<T: Send + Sync + 'static>(&self) -> Option<State<'_, T>> {
        get_state::<T>()
    }

    fn path(&self) -> &path::PathResolver<R> {
        &self.__handle().path
    }

    fn get_window(&self, _label: &str) -> Option<Window<R>> {
        None
    }

    fn get_webview(&self, _label: &str) -> Option<Webview<R>> {
        None
    }

    fn get_webview_window(&self, _label: &str) -> Option<WebviewWindow<R>> {
        None
    }

    fn get_focused_window(&self) -> Option<Window<R>> {
        None
    }

    fn windows(&self) -> HashMap<String, Window<R>> {
        HashMap::new()
    }

    fn webviews(&self) -> HashMap<String, Webview<R>> {
        HashMap::new()
    }

    fn webview_windows(&self) -> HashMap<String, WebviewWindow<R>> {
        HashMap::new()
    }
}

pub trait Emitter<R: Runtime>: ManagerBase<R> {
    fn emit<S: Serialize + Clone>(&self, event: &str, payload: S) -> Result<()> {
        event::emit(event, None, serde_json::to_string(&payload)?);
        Ok(())
    }

    fn emit_str(&self, event: &str, payload: String) -> Result<()> {
        event::emit(event, None, payload);
        Ok(())
    }

    fn emit_to<I, S>(&self, target: I, event: &str, payload: S) -> Result<()>
    where
        I: Into<event::EventTarget>,
        S: Serialize + Clone,
    {
        let target = target.into();
        event::emit(event, Some(target.label_or_any()), serde_json::to_string(&payload)?);
        Ok(())
    }

    fn emit_str_to<I: Into<event::EventTarget>>(
        &self,
        target: I,
        event: &str,
        payload: String,
    ) -> Result<()> {
        let target = target.into();
        event::emit(event, Some(target.label_or_any()), payload);
        Ok(())
    }

    fn emit_filter<S, F>(&self, event: &str, payload: S, _filter: F) -> Result<()>
    where
        S: Serialize + Clone,
        F: Fn(&event::EventTarget) -> bool,
    {
        self.emit(event, payload)
    }
}

pub trait Listener<R: Runtime>: ManagerBase<R> {
    fn listen<F>(&self, event: impl Into<String>, handler: F) -> event::EventId
    where
        F: Fn(event::Event) + Send + 'static,
    {
        event::listen(event.into(), false, event::Handler::Many(Mutex::new(Box::new(handler))))
    }

    fn once<F>(&self, event: impl Into<String>, handler: F) -> event::EventId
    where
        F: FnOnce(event::Event) + Send + 'static,
    {
        event::listen(
            event.into(),
            false,
            event::Handler::Once(Mutex::new(Some(Box::new(handler)))),
        )
    }

    fn unlisten(&self, id: event::EventId) {
        event::unlisten(id);
    }

    fn listen_any<F>(&self, event: impl Into<String>, handler: F) -> event::EventId
    where
        F: Fn(event::Event) + Send + 'static,
    {
        event::listen(event.into(), true, event::Handler::Many(Mutex::new(Box::new(handler))))
    }

    fn once_any<F>(&self, event: impl Into<String>, handler: F) -> event::EventId
    where
        F: FnOnce(event::Event) + Send + 'static,
    {
        event::listen(
            event.into(),
            true,
            event::Handler::Once(Mutex::new(Some(Box::new(handler)))),
        )
    }
}

macro_rules! manager_for {
    ($($ty:ident),*) => {$(
        impl<R: Runtime> Manager<R> for $ty<R> {}
        impl<R: Runtime> Emitter<R> for $ty<R> {}
        impl<R: Runtime> Listener<R> for $ty<R> {}
    )*};
}

impl<R: Runtime> ManagerBase<R> for AppHandle<R> {
    fn __handle(&self) -> &AppHandle<R> {
        self
    }
}

impl<R: Runtime> ManagerBase<R> for App<R> {
    fn __handle(&self) -> &AppHandle<R> {
        &self.handle
    }
}

impl<R: Runtime> ManagerBase<R> for Window<R> {
    fn __handle(&self) -> &AppHandle<R> {
        &self.app
    }
}

impl<R: Runtime> ManagerBase<R> for Webview<R> {
    fn __handle(&self) -> &AppHandle<R> {
        &self.app
    }
}

impl<R: Runtime> ManagerBase<R> for WebviewWindow<R> {
    fn __handle(&self) -> &AppHandle<R> {
        &self.app
    }
}

manager_for!(AppHandle, App, Window, Webview, WebviewWindow);

// ---------------------------------------------------------------------------
// Events

pub mod event {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    pub type EventId = u32;

    /// An event delivered to a Rust listener.
    #[derive(Debug, Clone)]
    pub struct Event {
        id: EventId,
        data: String,
    }

    impl Event {
        pub fn id(&self) -> EventId {
            self.id
        }

        pub fn payload(&self) -> &str {
            &self.data
        }
    }

    /// Who an event is for.
    #[derive(Debug, Clone, PartialEq, Eq)]
    #[non_exhaustive]
    pub enum EventTarget {
        Any,
        AnyLabel { label: String },
        App,
        Window { label: String },
        Webview { label: String },
        WebviewWindow { label: String },
    }

    impl EventTarget {
        pub fn any() -> Self {
            EventTarget::Any
        }

        pub fn app() -> Self {
            EventTarget::App
        }

        pub fn labeled(label: impl Into<String>) -> Self {
            EventTarget::AnyLabel { label: label.into() }
        }

        pub fn window(label: impl Into<String>) -> Self {
            EventTarget::Window { label: label.into() }
        }

        pub fn webview(label: impl Into<String>) -> Self {
            EventTarget::Webview { label: label.into() }
        }

        pub fn webview_window(label: impl Into<String>) -> Self {
            EventTarget::WebviewWindow { label: label.into() }
        }

        pub(crate) fn label_or_any(&self) -> String {
            match self {
                EventTarget::Any => "*".to_string(),
                EventTarget::App => "app".to_string(),
                EventTarget::AnyLabel { label }
                | EventTarget::Window { label }
                | EventTarget::Webview { label }
                | EventTarget::WebviewWindow { label } => label.clone(),
            }
        }
    }

    impl From<&str> for EventTarget {
        fn from(label: &str) -> Self {
            EventTarget::labeled(label)
        }
    }

    impl From<String> for EventTarget {
        fn from(label: String) -> Self {
            EventTarget::labeled(label)
        }
    }

    impl From<&String> for EventTarget {
        fn from(label: &String) -> Self {
            EventTarget::labeled(label.clone())
        }
    }

    pub(crate) enum Handler {
        Many(Mutex<Box<dyn Fn(Event) + Send>>),
        Once(Mutex<Option<Box<dyn FnOnce(Event) + Send>>>),
    }

    struct Listener {
        id: EventId,
        event: String,
        /// `listen_any`: also hears events sent to one label.
        any: bool,
        handler: Handler,
    }

    fn listeners() -> &'static Mutex<Vec<Arc<Listener>>> {
        static LISTENERS: OnceLock<Mutex<Vec<Arc<Listener>>>> = OnceLock::new();
        LISTENERS.get_or_init(Default::default)
    }

    static NEXT_ID: AtomicU32 = AtomicU32::new(1);

    pub(crate) fn listen(event: String, any: bool, handler: Handler) -> EventId {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut all) = listeners().lock() {
            all.push(Arc::new(Listener { id, event, any, handler }));
        }
        id
    }

    pub(crate) fn unlisten(id: EventId) {
        if let Ok(mut all) = listeners().lock() {
            all.retain(|l| l.id != id);
        }
    }

    /// Sends an event to the host sink, then to the Rust listeners that hear
    /// it. A broadcast reaches every listener; an event for one label reaches
    /// only `listen_any` listeners, as in Tauri, where `listen` on the app
    /// hears app-wide events only.
    pub(crate) fn emit(event: &str, target: Option<String>, payload_json: String) {
        let hearing: Vec<Arc<Listener>> = match listeners().lock() {
            Ok(all) => all
                .iter()
                .filter(|l| l.event == event && (target.is_none() || l.any))
                .cloned()
                .collect(),
            Err(_) => Vec::new(),
        };
        for listener in hearing {
            let delivered = Event {
                id: listener.id,
                data: payload_json.clone(),
            };
            match &listener.handler {
                Handler::Many(f) => {
                    if let Ok(f) = f.lock() {
                        f(delivered);
                    }
                }
                Handler::Once(f) => {
                    let taken = f.lock().ok().and_then(|mut f| f.take());
                    if let Some(f) = taken {
                        unlisten(listener.id);
                        f(delivered);
                    }
                }
            }
        }
        if let Some(h) = host() {
            h.sink.event(event, payload_json, target);
        }
    }
}

pub use event::{Event, EventId, EventTarget};

// ---------------------------------------------------------------------------
// async_runtime

pub mod async_runtime {
    use super::{Error, Result};
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::OnceLock;
    use std::task::{Context, Poll};

    /// The one multi-thread runtime every task runs on.
    #[doc(hidden)]
    pub fn runtime() -> &'static tokio::runtime::Runtime {
        static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
        RUNTIME.get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_name("sn-core")
                .build()
                .expect("failed to start the core's tokio runtime")
        })
    }

    pub fn handle() -> tokio::runtime::Handle {
        runtime().handle().clone()
    }

    pub fn spawn<F>(task: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        JoinHandle(runtime().spawn(task))
    }

    pub fn spawn_blocking<F, R>(func: F) -> JoinHandle<R>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        JoinHandle(runtime().spawn_blocking(func))
    }

    pub fn block_on<F: Future>(task: F) -> F::Output {
        runtime().block_on(task)
    }

    /// A spawned task. Awaiting it yields the task's output, or an error when
    /// the task panicked or was aborted.
    #[derive(Debug)]
    pub struct JoinHandle<T>(tokio::task::JoinHandle<T>);

    impl<T> JoinHandle<T> {
        pub fn abort(&self) {
            self.0.abort();
        }

        pub fn inner(&self) -> &tokio::task::JoinHandle<T> {
            &self.0
        }
    }

    impl<T> Future for JoinHandle<T> {
        type Output = Result<T>;
        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            Pin::new(&mut self.0)
                .poll(cx)
                .map(|r| r.map_err(|e| Error::Join(e.to_string())))
        }
    }
}

// ---------------------------------------------------------------------------
// path

pub mod path {
    use super::{host, Error, Result, Runtime};
    use std::marker::PhantomData;
    use std::path::{Path, PathBuf};

    /// Resolves the app's directories from the ones the shell installed.
    pub struct PathResolver<R: Runtime>(pub(crate) PhantomData<fn() -> R>);

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[non_exhaustive]
    pub enum BaseDirectory {
        AppData,
        AppLocalData,
        AppConfig,
        AppCache,
        AppLog,
        Temp,
    }

    impl<R: Runtime> PathResolver<R> {
        fn dirs() -> Result<&'static super::HostDirs> {
            host().map(|h| &h.dirs).ok_or(Error::HostNotInstalled)
        }

        pub fn app_data_dir(&self) -> Result<PathBuf> {
            Ok(Self::dirs()?.data.clone())
        }

        pub fn app_local_data_dir(&self) -> Result<PathBuf> {
            Ok(Self::dirs()?.data.clone())
        }

        pub fn app_config_dir(&self) -> Result<PathBuf> {
            Ok(Self::dirs()?.data.clone())
        }

        pub fn app_cache_dir(&self) -> Result<PathBuf> {
            Ok(Self::dirs()?.cache.clone())
        }

        pub fn app_log_dir(&self) -> Result<PathBuf> {
            Ok(Self::dirs()?.logs.clone())
        }

        pub fn temp_dir(&self) -> Result<PathBuf> {
            Ok(std::env::temp_dir())
        }

        pub fn resource_dir(&self) -> Result<PathBuf> {
            Err(Error::Unsupported("the resource directory"))
        }

        pub fn resolve<P: AsRef<Path>>(&self, path: P, base: BaseDirectory) -> Result<PathBuf> {
            let dir = match base {
                BaseDirectory::AppData => self.app_data_dir()?,
                BaseDirectory::AppLocalData => self.app_local_data_dir()?,
                BaseDirectory::AppConfig => self.app_config_dir()?,
                BaseDirectory::AppCache => self.app_cache_dir()?,
                BaseDirectory::AppLog => self.app_log_dir()?,
                BaseDirectory::Temp => self.temp_dir()?,
            };
            Ok(dir.join(path))
        }
    }
}

// ---------------------------------------------------------------------------
// ipc

pub mod ipc {
    use super::{AppHandle, Result};
    use serde::Serialize;
    use std::marker::PhantomData;

    pub type CmdResult = std::result::Result<serde_json::Value, serde_json::Value>;
    pub type CmdFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = CmdResult> + Send>>;
    type Lookup = dyn Fn(&str, AppHandle, serde_json::Value) -> Option<CmdFuture> + Send + Sync;

    /// What `generate_handler!` builds: command name to entry point.
    pub struct Handler(pub(crate) Box<Lookup>);

    impl Handler {
        #[doc(hidden)]
        pub fn __new<F>(lookup: F) -> Self
        where
            F: Fn(&str, AppHandle, serde_json::Value) -> Option<CmdFuture> + Send + Sync + 'static,
        {
            Handler(Box::new(lookup))
        }
    }

    #[derive(Debug, Clone)]
    pub enum InvokeBody {
        Json(serde_json::Value),
        Raw(Vec<u8>),
    }

    /// The raw invoke, for commands that read the body or headers themselves.
    pub struct Request<'a> {
        body: &'a InvokeBody,
        headers: &'a http::HeaderMap,
    }

    impl<'a> Request<'a> {
        pub fn body(&self) -> &InvokeBody {
            self.body
        }

        pub fn headers(&self) -> &http::HeaderMap {
            self.headers
        }

        pub(crate) fn new(body: &'a InvokeBody, headers: &'a http::HeaderMap) -> Self {
            Request { body, headers }
        }
    }

    /// A binary command result.
    #[derive(Debug, Clone)]
    pub struct Response {
        body: Vec<u8>,
    }

    impl Response {
        pub fn new(body: impl Into<Vec<u8>>) -> Self {
            Response { body: body.into() }
        }
    }

    impl Serialize for Response {
        fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
            self.body.serialize(s)
        }
    }

    /// A stream of values back to the caller. Each `send` is an event named
    /// `__channel:<id>` with the value as its payload.
    pub struct Channel<T = serde_json::Value> {
        id: u64,
        _value: PhantomData<fn(T)>,
    }

    impl<T> Clone for Channel<T> {
        fn clone(&self) -> Self {
            Channel {
                id: self.id,
                _value: PhantomData,
            }
        }
    }

    impl<T> std::fmt::Debug for Channel<T> {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "Channel({})", self.id)
        }
    }

    impl<T: Serialize> Channel<T> {
        pub fn id(&self) -> u64 {
            self.id
        }

        pub fn send(&self, data: T) -> Result<()> {
            let json = serde_json::to_string(&data)?;
            super::event::emit(&format!("__channel:{}", self.id), None, json);
            Ok(())
        }
    }

    impl<T> Channel<T> {
        pub(crate) fn with_id(id: u64) -> Self {
            Channel {
                id,
                _value: PhantomData,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Windows and webviews (none exist)

pub mod window {
    use super::{AppHandle, Error, Result, Runtime, Wry};

    /// A window. None exist on iOS; commands that take one get a placeholder
    /// labelled `main` whose calls do nothing.
    pub struct Window<R: Runtime = Wry> {
        pub(crate) label: String,
        pub(crate) app: AppHandle<R>,
    }

    impl<R: Runtime> Clone for Window<R> {
        fn clone(&self) -> Self {
            Window {
                label: self.label.clone(),
                app: self.app.clone(),
            }
        }
    }

    impl<R: Runtime> std::fmt::Debug for Window<R> {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "Window({})", self.label)
        }
    }

    impl<R: Runtime> Window<R> {
        pub(crate) fn placeholder(app: AppHandle<R>) -> Self {
            Window {
                label: "main".to_string(),
                app,
            }
        }

        pub fn label(&self) -> &str {
            &self.label
        }

        pub fn is_visible(&self) -> Result<bool> {
            Ok(true)
        }

        pub fn is_minimized(&self) -> Result<bool> {
            Ok(false)
        }

        pub fn is_focused(&self) -> Result<bool> {
            Ok(true)
        }

        pub fn show(&self) -> Result<()> {
            Ok(())
        }

        pub fn hide(&self) -> Result<()> {
            Ok(())
        }

        pub fn set_focus(&self) -> Result<()> {
            Ok(())
        }

        pub fn close(&self) -> Result<()> {
            Err(Error::Unsupported("closing a window"))
        }

        pub fn destroy(&self) -> Result<()> {
            Err(Error::Unsupported("destroying a window"))
        }

        pub fn scale_factor(&self) -> Result<f64> {
            Ok(1.0)
        }

        pub fn inner_size(&self) -> Result<PhysicalSize<u32>> {
            Err(Error::Unsupported("a window size"))
        }

        pub fn outer_size(&self) -> Result<PhysicalSize<u32>> {
            Err(Error::Unsupported("a window size"))
        }

        pub fn outer_position(&self) -> Result<PhysicalPosition<i32>> {
            Err(Error::Unsupported("a window position"))
        }

        pub fn primary_monitor(&self) -> Result<Option<Monitor>> {
            Ok(None)
        }

        pub fn current_monitor(&self) -> Result<Option<Monitor>> {
            Ok(None)
        }

        pub fn available_monitors(&self) -> Result<Vec<Monitor>> {
            Ok(Vec::new())
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct Color(pub u8, pub u8, pub u8, pub u8);

    #[derive(Debug, Clone)]
    pub struct Monitor {
        pub(crate) name: Option<String>,
        pub(crate) size: PhysicalSize<u32>,
        pub(crate) position: PhysicalPosition<i32>,
        pub(crate) scale_factor: f64,
    }

    impl Monitor {
        pub fn name(&self) -> Option<&String> {
            self.name.as_ref()
        }

        pub fn size(&self) -> &PhysicalSize<u32> {
            &self.size
        }

        pub fn position(&self) -> &PhysicalPosition<i32> {
            &self.position
        }

        pub fn scale_factor(&self) -> f64 {
            self.scale_factor
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
    pub struct PhysicalSize<T> {
        pub width: T,
        pub height: T,
    }

    impl<T> PhysicalSize<T> {
        pub fn new(width: T, height: T) -> Self {
            PhysicalSize { width, height }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
    pub struct PhysicalPosition<T> {
        pub x: T,
        pub y: T,
    }

    impl<T> PhysicalPosition<T> {
        pub fn new(x: T, y: T) -> Self {
            PhysicalPosition { x, y }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
    pub struct LogicalSize<T> {
        pub width: T,
        pub height: T,
    }

    impl<T> LogicalSize<T> {
        pub fn new(width: T, height: T) -> Self {
            LogicalSize { width, height }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
    pub struct LogicalPosition<T> {
        pub x: T,
        pub y: T,
    }

    impl<T> LogicalPosition<T> {
        pub fn new(x: T, y: T) -> Self {
            LogicalPosition { x, y }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq)]
    pub enum Size {
        Physical(PhysicalSize<u32>),
        Logical(LogicalSize<f64>),
    }

    impl From<PhysicalSize<u32>> for Size {
        fn from(s: PhysicalSize<u32>) -> Self {
            Size::Physical(s)
        }
    }

    impl From<LogicalSize<f64>> for Size {
        fn from(s: LogicalSize<f64>) -> Self {
            Size::Logical(s)
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq)]
    pub enum Position {
        Physical(PhysicalPosition<i32>),
        Logical(LogicalPosition<f64>),
    }

    impl From<PhysicalPosition<i32>> for Position {
        fn from(p: PhysicalPosition<i32>) -> Self {
            Position::Physical(p)
        }
    }

    impl From<LogicalPosition<f64>> for Position {
        fn from(p: LogicalPosition<f64>) -> Self {
            Position::Logical(p)
        }
    }

    #[derive(Debug, Clone, Default)]
    pub struct CloseRequestApi(());

    impl CloseRequestApi {
        pub fn prevent_close(&self) {}
    }

    #[derive(Debug, Clone)]
    #[non_exhaustive]
    pub enum WindowEvent {
        Resized(PhysicalSize<u32>),
        Moved(PhysicalPosition<i32>),
        CloseRequested { api: CloseRequestApi },
        Destroyed,
        Focused(bool),
        ScaleFactorChanged {
            scale_factor: f64,
            new_inner_size: PhysicalSize<u32>,
        },
    }
}

pub mod webview {
    use super::{AppHandle, Error, Manager, Result, Runtime, Url, Wry};
    use std::marker::PhantomData;
    use std::path::PathBuf;

    #[derive(Debug, Clone)]
    #[non_exhaustive]
    pub enum WebviewUrl {
        External(Url),
        App(PathBuf),
        CustomProtocol(Url),
    }

    macro_rules! placeholder_view {
        ($name:ident, $what:literal) => {
            #[doc = concat!("A ", $what, ". None exist on iOS; commands that take one get a placeholder labelled `main` whose calls do nothing.")]
            pub struct $name<R: Runtime = Wry> {
                pub(crate) label: String,
                pub(crate) app: AppHandle<R>,
            }

            impl<R: Runtime> Clone for $name<R> {
                fn clone(&self) -> Self {
                    $name {
                        label: self.label.clone(),
                        app: self.app.clone(),
                    }
                }
            }

            impl<R: Runtime> std::fmt::Debug for $name<R> {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    write!(f, concat!(stringify!($name), "({})"), self.label)
                }
            }

            impl<R: Runtime> $name<R> {
                pub(crate) fn placeholder(app: AppHandle<R>) -> Self {
                    $name {
                        label: "main".to_string(),
                        app,
                    }
                }

                pub fn label(&self) -> &str {
                    &self.label
                }

                pub fn eval(&self, _js: impl Into<String>) -> Result<()> {
                    Err(Error::Unsupported("running script in a webview"))
                }

                pub fn url(&self) -> Result<Url> {
                    Err(Error::Unsupported("a webview URL"))
                }

                pub fn navigate(&self, _url: Url) -> Result<()> {
                    Err(Error::Unsupported("navigating a webview"))
                }

                pub fn window(&self) -> super::Window<R> {
                    super::Window::placeholder(self.app.clone())
                }

                pub fn is_visible(&self) -> Result<bool> {
                    Ok(true)
                }

                pub fn is_minimized(&self) -> Result<bool> {
                    Ok(false)
                }

                pub fn is_focused(&self) -> Result<bool> {
                    Ok(true)
                }

                pub fn show(&self) -> Result<()> {
                    Ok(())
                }

                pub fn hide(&self) -> Result<()> {
                    Ok(())
                }

                pub fn set_focus(&self) -> Result<()> {
                    Ok(())
                }

                pub fn close(&self) -> Result<()> {
                    Err(Error::Unsupported("closing a window"))
                }

                pub fn destroy(&self) -> Result<()> {
                    Err(Error::Unsupported("destroying a window"))
                }

                pub fn scale_factor(&self) -> Result<f64> {
                    Ok(1.0)
                }

                pub fn inner_size(&self) -> Result<super::PhysicalSize<u32>> {
                    Err(Error::Unsupported("a window size"))
                }

                pub fn outer_size(&self) -> Result<super::PhysicalSize<u32>> {
                    Err(Error::Unsupported("a window size"))
                }

                pub fn outer_position(&self) -> Result<super::PhysicalPosition<i32>> {
                    Err(Error::Unsupported("a window position"))
                }

                pub fn primary_monitor(&self) -> Result<Option<super::window::Monitor>> {
                    Ok(None)
                }

                pub fn current_monitor(&self) -> Result<Option<super::window::Monitor>> {
                    Ok(None)
                }

                pub fn available_monitors(&self) -> Result<Vec<super::window::Monitor>> {
                    Ok(Vec::new())
                }

                pub fn cookies_for_url(&self, _url: Url) -> Result<Vec<Cookie<'static>>> {
                    Ok(Vec::new())
                }

                pub fn cookies(&self) -> Result<Vec<Cookie<'static>>> {
                    Ok(Vec::new())
                }
            }
        };
    }

    placeholder_view!(Webview, "webview");
    placeholder_view!(WebviewWindow, "webview window");

    /// A cookie a webview holds. No webview holds any on iOS.
    #[derive(Debug, Clone)]
    pub struct Cookie<'a> {
        name: std::borrow::Cow<'a, str>,
        value: std::borrow::Cow<'a, str>,
    }

    impl Cookie<'_> {
        pub fn name(&self) -> &str {
            &self.name
        }

        pub fn value(&self) -> &str {
            &self.value
        }

        pub fn domain(&self) -> Option<&str> {
            None
        }

        pub fn path(&self) -> Option<&str> {
            None
        }

        pub fn secure(&self) -> Option<bool> {
            None
        }

        pub fn http_only(&self) -> Option<bool> {
            None
        }
    }

    /// What a page's `window.open` asks for.
    #[derive(Debug, Clone, Default)]
    pub struct NewWindowFeatures(());

    pub enum NewWindowResponse<R: Runtime = Wry> {
        Allow,
        Deny,
        Create { window: WebviewWindow<R> },
    }

    #[derive(Debug)]
    #[non_exhaustive]
    pub enum DownloadEvent<'a> {
        Requested {
            url: Url,
            destination: &'a mut PathBuf,
        },
        Finished {
            url: Url,
            path: Option<PathBuf>,
            success: bool,
        },
    }

    /// Builds a window. Building always fails: the iOS shell has no windows.
    pub struct WebviewWindowBuilder<'a, R: Runtime = Wry, M: Manager<R> = AppHandle<R>> {
        label: String,
        _manager: PhantomData<(&'a M, fn() -> R)>,
    }

    macro_rules! ignored_options {
        ($($name:ident($($arg:ident: $ty:ty),*)),* $(,)?) => {$(
            pub fn $name(self, $($arg: $ty),*) -> Self {
                let _ = ($($arg,)*);
                self
            }
        )*};
    }

    impl<'a, R: Runtime, M: Manager<R>> WebviewWindowBuilder<'a, R, M> {
        pub fn new<L: Into<String>>(_manager: &'a M, label: L, _url: WebviewUrl) -> Self {
            WebviewWindowBuilder {
                label: label.into(),
                _manager: PhantomData,
            }
        }

        ignored_options!(
            title(_title: impl Into<String>),
            inner_size(_width: f64, _height: f64),
            min_inner_size(_width: f64, _height: f64),
            max_inner_size(_width: f64, _height: f64),
            position(_x: f64, _y: f64),
            center(),
            resizable(_resizable: bool),
            maximizable(_maximizable: bool),
            minimizable(_minimizable: bool),
            closable(_closable: bool),
            visible(_visible: bool),
            focused(_focused: bool),
            focusable(_focusable: bool),
            decorations(_decorations: bool),
            transparent(_transparent: bool),
            shadow(_shadow: bool),
            always_on_top(_always_on_top: bool),
            skip_taskbar(_skip: bool),
            incognito(_incognito: bool),
            devtools(_devtools: bool),
            user_agent(_user_agent: &str),
            initialization_script(_script: impl Into<String>),
            data_directory(_dir: PathBuf),
            data_store_identifier(_id: [u8; 16]),
            background_color(_color: super::window::Color),
        );

        pub fn on_navigation<F: Fn(&Url) -> bool + Send + 'static>(self, _f: F) -> Self {
            self
        }

        pub fn build(self) -> Result<WebviewWindow<R>> {
            log::warn!("[Headless] window {} not built: no windows on iOS", self.label);
            Err(Error::Unsupported("building a window"))
        }
    }
}

pub mod image {
    /// An image. Only named so window-icon lookups type-check; none exist.
    #[derive(Debug, Clone)]
    pub struct Image<'a> {
        rgba: std::borrow::Cow<'a, [u8]>,
        width: u32,
        height: u32,
    }

    impl<'a> Image<'a> {
        pub fn new(rgba: &'a [u8], width: u32, height: u32) -> Self {
            Image {
                rgba: std::borrow::Cow::Borrowed(rgba),
                width,
                height,
            }
        }

        pub fn rgba(&self) -> &[u8] {
            &self.rgba
        }

        pub fn width(&self) -> u32 {
            self.width
        }

        pub fn height(&self) -> u32 {
            self.height
        }
    }
}

pub mod plugin {
    use super::{Runtime, Wry};
    use std::marker::PhantomData;

    /// A plugin. Plugins are not registered on iOS.
    pub struct TauriPlugin<R: Runtime = Wry>(PhantomData<fn() -> R>);

    pub struct Builder<R: Runtime = Wry> {
        _runtime: PhantomData<fn() -> R>,
    }

    impl<R: Runtime> Builder<R> {
        pub fn new(_name: &'static str) -> Self {
            Builder { _runtime: PhantomData }
        }

        pub fn js_init_script(self, _script: String) -> Self {
            self
        }

        pub fn build(self) -> TauriPlugin<R> {
            TauriPlugin(PhantomData)
        }
    }
}

// ---------------------------------------------------------------------------
// What the command macros call

#[doc(hidden)]
pub mod __private {
    use super::ipc::{Channel, InvokeBody, Request};
    use super::{AppHandle, Runtime, State};
    use serde::de::DeserializeOwned;
    use serde::Serialize;

    pub use super::ipc::{CmdFuture, CmdResult};
    pub use http::HeaderMap;
    pub use serde_json::Value;

    fn error(message: String) -> Value {
        Value::String(message)
    }

    pub fn state<T: Send + Sync + 'static>(_app: &AppHandle) -> Result<State<'static, T>, Value> {
        super::get_state::<T>().ok_or_else(|| {
            error(format!(
                "state not managed for {}",
                std::any::type_name::<T>()
            ))
        })
    }

    pub fn app<R: Runtime>(_app: &AppHandle) -> Result<AppHandle<R>, Value> {
        Ok(AppHandle::new())
    }

    pub trait Placeholder {
        fn placeholder() -> Self;
    }

    impl<R: Runtime> Placeholder for super::Window<R> {
        fn placeholder() -> Self {
            super::Window::placeholder(AppHandle::new())
        }
    }

    impl<R: Runtime> Placeholder for super::Webview<R> {
        fn placeholder() -> Self {
            super::Webview::placeholder(AppHandle::new())
        }
    }

    impl<R: Runtime> Placeholder for super::WebviewWindow<R> {
        fn placeholder() -> Self {
            super::WebviewWindow::placeholder(AppHandle::new())
        }
    }

    pub fn placeholder<P: Placeholder>(_app: &AppHandle) -> Result<P, Value> {
        Ok(P::placeholder())
    }

    /// A channel argument: an id number, `"__CHANNEL__:<id>"`, or `{ "id": <id> }`.
    pub fn channel<T>(_app: &AppHandle, args: &Value, key: &str) -> Result<Channel<T>, Value> {
        let raw = args.get(key).unwrap_or(&Value::Null);
        let id = match raw {
            Value::Number(n) => n.as_u64(),
            Value::String(s) => s.strip_prefix("__CHANNEL__:").unwrap_or(s).parse().ok(),
            Value::Object(o) => o.get("id").and_then(Value::as_u64),
            _ => None,
        };
        id.map(Channel::with_id)
            .ok_or_else(|| error(format!("argument {key} is not a channel id")))
    }

    pub fn arg<T: DeserializeOwned>(args: &Value, key: &str) -> Result<T, Value> {
        let raw = args.get(key).cloned();
        let missing = raw.is_none();
        serde_json::from_value(raw.unwrap_or(Value::Null)).map_err(|e| {
            if missing {
                error(format!("missing required key {key}"))
            } else {
                error(format!("invalid argument {key}: {e}"))
            }
        })
    }

    pub fn body(args: &Value) -> InvokeBody {
        InvokeBody::Json(args.clone())
    }

    pub fn request<'a>(body: &'a InvokeBody, headers: &'a HeaderMap) -> Result<Request<'a>, Value> {
        Ok(Request::new(body, headers))
    }

    /// Runs a synchronous command off the async workers.
    pub async fn blocking<F>(f: F) -> CmdResult
    where
        F: FnOnce() -> CmdResult + Send + 'static,
    {
        match super::async_runtime::spawn_blocking(f).await {
            Ok(result) => result,
            Err(e) => Err(error(e.to_string())),
        }
    }

    pub struct ResultTag;
    pub struct PlainTag;

    /// Picked for commands that return `Result<T, E>`.
    pub trait ResultKind {
        fn __headless_kind(&self) -> ResultTag {
            ResultTag
        }
    }

    impl<T: Serialize, E: Serialize> ResultKind for Result<T, E> {}

    /// Picked for every other serializable return value.
    pub trait PlainKind {
        fn __headless_kind(&self) -> PlainTag {
            PlainTag
        }
    }

    impl<T: Serialize> PlainKind for &T {}

    fn to_value<T: Serialize>(value: T) -> Value {
        serde_json::to_value(value).unwrap_or_else(|e| error(format!("result does not serialize: {e}")))
    }

    impl ResultTag {
        pub fn __headless_into<T: Serialize, E: Serialize>(self, result: Result<T, E>) -> CmdResult {
            match result {
                Ok(value) => Ok(to_value(value)),
                Err(e) => Err(to_value(e)),
            }
        }
    }

    impl PlainTag {
        pub fn __headless_into<T: Serialize>(self, value: T) -> CmdResult {
            Ok(to_value(value))
        }
    }
}

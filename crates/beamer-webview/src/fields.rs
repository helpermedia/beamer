//! What a model declares for its web UI: its senders and events.
//!
//! A sender or an event is a field of the model with an attribute that
//! gives its name, beside the parameters and `State` fields, typed by what
//! it carries:
//!
//! ```ignore
//! #[beamer::model]
//! struct MeterModel {
//!     /// The levels the editor's meters show.
//!     #[value(id = "levels")]
//!     levels: WebViewValue<[f32; 2]>,
//!
//!     /// Every point of the scrolling history.
//!     #[stream(id = "history")]
//!     history: WebViewStream<f32>,
//!
//!     /// The sample rate, when the host prepares the plugin again.
//!     #[event(id = "sampleRate")]
//!     sample_rate_event: WebViewEvent<f64>,
//!
//!     events: WebViewEventQueue,
//! }
//!
//! // In prepare(), before `self` moves into the processor:
//! MeterProcessor {
//!     levels: self.levels.sender([0.0; 2]),
//!     history: self.history.sender(64),
//!     model: self,
//! }
//! ```
//!
//! The name lives in one place: the page reads the same name from the
//! model's types. A value of the wrong type does not compile on either
//! side. In a nested group the name gets the group's field name in front,
//! as the paths of parameters and `State` fields do (`note/pressed`).
//!
//! `#[beamer::model]` connects each field to the model's
//! [`WebViewEventQueue`] when it assigns the ids, so a model without one
//! has no way to deliver them: making a sender or emitting then panics in
//! debug builds.

use std::marker::PhantomData;

use serde::Serialize;

use crate::event_queue::WebViewEventQueue;
use crate::stream_sender::WebViewStreamSender;
use crate::value_sender::WebViewValueSender;

/// Where a declared field delivers: its name and the model's queue, once
/// `#[beamer::model]` connected it.
#[derive(Default)]
struct Connection {
    // The name the page knows it by: the attribute's id behind the names of
    // the nested groups.
    name: String,
    queue: Option<WebViewEventQueue>,
    // The queue of a field that was never connected, in release builds:
    // made with the one report.
    #[cfg(not(debug_assertions))]
    unconnected: std::sync::OnceLock<WebViewEventQueue>,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field("name", &self.name)
            .field("connected", &self.queue.is_some())
            .finish()
    }
}

impl Connection {
    /// Connected to `queue` under `name`.
    // Release builds have one more field.
    #[allow(clippy::needless_update)]
    fn new(name: String, queue: &WebViewEventQueue) -> Self {
        Self { name, queue: Some(queue.clone()), ..Self::default() }
    }

    /// The queue to deliver through. A field that was never connected
    /// panics in debug builds. In release builds it reports once and gets
    /// a queue of its own that no editor reads.
    #[cfg_attr(debug_assertions, track_caller)]
    fn queue(&self, kind: &str) -> WebViewEventQueue {
        if let Some(queue) = &self.queue {
            return queue.clone();
        }
        let message = || {
            format!(
                "A {kind} field delivers nothing: it is not connected to a queue. Give the model \
                 a `WebViewEventQueue` field (a nested group's counts too); a model with \
                 `no_default` calls `assign_ids(\"\")` in its `Default`."
            )
        };
        #[cfg(debug_assertions)]
        panic!("{}", message());
        #[cfg(not(debug_assertions))]
        self.unconnected
            .get_or_init(|| {
                let message = message();
                log::error!("{message}");
                eprintln!("{message}");
                WebViewEventQueue::reported()
            })
            .clone()
    }
}

/// A value sender the model declares with `#[value(id = "...")]`: the
/// newest value of something the audio thread measures, such as a meter.
///
/// The page reads it with `senderValue(name)` of beamer-api. See
/// [`WebViewValueSender`] for how sending works.
#[derive(Debug)]
pub struct WebViewValue<T> {
    connection: Connection,
    // The type of what it sends. A function pointer keeps the field `Send`
    // and `Sync` whatever `T` is: the field holds no value.
    _value: PhantomData<fn(T)>,
}

impl<T> Default for WebViewValue<T> {
    fn default() -> Self {
        Self { connection: Connection::default(), _value: PhantomData }
    }
}

impl<T> WebViewValue<T> {
    /// The name the page knows the sender by.
    pub fn name(&self) -> &str {
        &self.connection.name
    }

    /// Connect the field to the model's queue under `name`.
    /// `#[beamer::model]` calls this; plugin code does not.
    #[doc(hidden)]
    pub fn connect(&mut self, name: String, queue: &WebViewEventQueue) {
        self.connection = Connection::new(name, queue);
    }
}

impl<T: Clone + Serialize + Send + 'static> WebViewValue<T> {
    /// Create the sender for the audio thread, with the value before the
    /// first send.
    ///
    /// Call this in `prepare()` and keep the sender in the processor. Takes
    /// a lock and allocates: not for the audio thread. `initial` also sets
    /// the memory a value fills (the length of a `Vec`, for example). See
    /// [`WebViewValueSender`].
    #[cfg_attr(debug_assertions, track_caller)]
    pub fn sender(&self, initial: T) -> WebViewValueSender<T> {
        self.connection.queue("WebViewValue").value_sender(&self.connection.name, initial)
    }
}

/// A stream sender the model declares with `#[stream(id = "...")]`: every
/// value in order, such as the points of a scrolling history.
///
/// The page receives the values since the last tick as an array through
/// `runtime().on(name, ...)` of beamer-api. See [`WebViewStreamSender`].
#[derive(Debug)]
pub struct WebViewStream<T> {
    connection: Connection,
    // The type of what it sends, as for `WebViewValue`.
    _value: PhantomData<fn(T)>,
}

impl<T> Default for WebViewStream<T> {
    fn default() -> Self {
        Self { connection: Connection::default(), _value: PhantomData }
    }
}

impl<T> WebViewStream<T> {
    /// The name the page knows the stream by.
    pub fn name(&self) -> &str {
        &self.connection.name
    }

    /// Connect the field to the model's queue under `name`.
    /// `#[beamer::model]` calls this; plugin code does not.
    #[doc(hidden)]
    pub fn connect(&mut self, name: String, queue: &WebViewEventQueue) {
        self.connection = Connection::new(name, queue);
    }
}

impl<T: Copy + Serialize + Send + 'static> WebViewStream<T> {
    /// Create the sender for the audio thread, with room for `capacity`
    /// values between two ticks of the editor; more are dropped.
    ///
    /// Call this in `prepare()` and keep the sender in the processor. Takes
    /// a lock and allocates: not for the audio thread. See
    /// [`WebViewStreamSender`].
    ///
    /// # Panics
    ///
    /// When `capacity` is 0.
    #[cfg_attr(debug_assertions, track_caller)]
    pub fn sender(&self, capacity: usize) -> WebViewStreamSender<T> {
        self.connection.queue("WebViewStream").stream_sender(&self.connection.name, capacity)
    }
}

/// An event the model declares with `#[event(id = "...")]`: a notification
/// for an open editor, such as a sample that finished loading.
///
/// The page receives it through `runtime().on(name, ...)` of beamer-api.
/// While no editor is open, events are dropped: see
/// [`WebViewEventQueue`] for what an event is for.
#[derive(Debug)]
pub struct WebViewEvent<T> {
    connection: Connection,
    // Whether data with a number JSON does not hold as it is was reported
    // already.
    #[cfg(debug_assertions)]
    number_reported: std::sync::atomic::AtomicBool,
    // The type of the data, as for `WebViewValue`.
    _data: PhantomData<fn(T)>,
}

impl<T> Default for WebViewEvent<T> {
    fn default() -> Self {
        Self {
            connection: Connection::default(),
            #[cfg(debug_assertions)]
            number_reported: std::sync::atomic::AtomicBool::new(false),
            _data: PhantomData,
        }
    }
}

impl<T> WebViewEvent<T> {
    /// The name the page knows the event by.
    pub fn name(&self) -> &str {
        &self.connection.name
    }

    /// Connect the field to the model's queue under `name`.
    /// `#[beamer::model]` calls this; plugin code does not.
    #[doc(hidden)]
    pub fn connect(&mut self, name: String, queue: &WebViewEventQueue) {
        self.connection = Connection::new(name, queue);
    }
}

impl<T: Serialize> WebViewEvent<T> {
    /// Send the event with its data to the open editor.
    ///
    /// Dropped while no editor is open. Not for the audio thread: this
    /// allocates and takes a lock. The audio thread sends through a
    /// [`WebViewValue`] or a [`WebViewStream`].
    #[cfg_attr(debug_assertions, track_caller)]
    pub fn emit(&self, data: &T) {
        #[cfg(debug_assertions)]
        if !self.number_reported.load(std::sync::atomic::Ordering::Relaxed) {
            if let Some(found) = beamer_core::json_check::unfaithful_number(data) {
                if !self.number_reported.swap(true, std::sync::atomic::Ordering::Relaxed) {
                    let what = format!("The event {:?}", self.connection.name);
                    beamer_core::json_check::report(&what, &found);
                }
            }
        }
        self.connection.queue("WebViewEvent").emit(&self.connection.name, data);
    }
}

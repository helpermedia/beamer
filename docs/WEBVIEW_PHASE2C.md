# Phase 2C: IPC & Parameter Binding

Bidirectional communication between the WebView UI and the Rust plugin.
Automatic parameter synchronization with host notification.

## Overview

Phase 2B serves static web content. The UI has no way to read or write
plugin parameters, invoke Rust functions or receive events from the
audio processor. Phase 2C adds:

1. **Parameter synchronization** - automatic two-way binding between
   the host's parameter tree and the JS UI
2. **Invoke pattern** (JS -> Rust) - call named Rust handlers from
   JavaScript and receive results as Promises
3. **Event emission** (Rust -> JS) - push named events from the plugin
   to the UI
4. **`window.__BEAMER__`** - injected JavaScript runtime that provides
   the public API

Phase 2C targets macOS only. Windows notes are included for reference
where relevant but implementation is deferred.

## Architecture

Two platform channels form the transport layer:

| Direction | macOS | Windows (future) |
|-----------|-------|-------------------|
| JS -> Rust | `WKScriptMessageHandler` | `WebMessageReceived` |
| Rust -> JS | `evaluateJavaScript(_:)` | `ExecuteScriptAsync` |

Everything above this layer (parameter sync, invoke, events) is a JSON
protocol running over the same two channels.

```
 ┌──────────────────────────────────┐
 │         JavaScript UI            │
 │  window.__BEAMER__.params.set()  │
 │  window.__BEAMER__.invoke()      │
 │  window.__BEAMER__.on()          │
 └──────────┬───────────────────────┘
            │ webkit.messageHandlers
            │ .beamer.postMessage(json)
            ▼
 ┌──────────────────────────────────┐
 │     WKScriptMessageHandler       │  ◄── JS -> Rust
 │     (BeamerMessageHandler)       │
 └──────────┬───────────────────────┘
            │ calls message_callback
            ▼
 ┌──────────────────────────────────┐
 │       IPC Dispatcher (Rust)      │
 │  routes: param:set, param:begin, │
 │  param:end, invoke, event        │
 └──────────┬───────────────────────┘
            │ evaluateJavaScript
            ▼
 ┌──────────────────────────────────┐
 │    window.__BEAMER__._onXxx()    │  ◄── Rust -> JS
 └──────────────────────────────────┘

 ┌──────────────────────────────────┐
 │     Parameter Sync Timer         │
 │  60 Hz poll, dirty-flag check,   │
 │  batched evaluateJavaScript      │
 └──────────────────────────────────┘
```

## Message Protocol

All messages are JSON objects with a `type` field.

### JS -> Rust

```json
{"type":"param:set","id":42,"value":0.75}
{"type":"param:begin","id":42}
{"type":"param:end","id":42}
{"type":"invoke","method":"loadPreset","args":["bright"],"callId":1}
{"type":"event","name":"waveformClicked","data":{"x":120,"y":45}}
```

Parameter `id` is the numeric `ParameterId` (u32 hash of the string
ID). The JavaScript runtime maps string IDs to numeric IDs internally
so plugin JS code uses strings.

### Rust -> JS

Delivered via `evaluateJavaScript`. Each call evaluates a function on
`window.__BEAMER__`:

```javascript
// Parameter value update (batched, one call per tick)
window.__BEAMER__._onParams({42:0.75,87:0.33})

// Parameter info dump (sent once on init)
window.__BEAMER__._onInit([
  {"id":42,"stringId":"gain","name":"Gain","min":-60,"max":12,
   "defaultValue":0.5,"value":0.75,"units":"dB","steps":0},
  ...
])

// Invoke response (success)
window.__BEAMER__._onResult(1,{"ok":"bright loaded"})

// Invoke response (error)
window.__BEAMER__._onResult(1,{"err":"preset not found"})

// Event from Rust
window.__BEAMER__._onEvent("spectrumData",[0.1,0.4,0.8,...])
```

## JavaScript API

An injected user script (at document start) creates
`window.__BEAMER__`:

```typescript
interface Beamer {
  /** Resolves when parameter init dump is received. */
  ready: Promise<void>;

  params: {
    /** Get normalized value (0-1) by string ID. */
    get(stringId: string): number;
    /** Set normalized value and notify the host. */
    set(stringId: string, value: number): void;
    /** Begin a parameter edit gesture (for host undo). */
    beginEdit(stringId: string): void;
    /** End a parameter edit gesture. */
    endEdit(stringId: string): void;
    /** Subscribe to value changes. Returns unsubscribe function. */
    on(stringId: string, callback: (value: number) => void): () => void;
    /** Get all parameter info objects. */
    all(): ParamInfo[];
    /** Get info for one parameter. */
    info(stringId: string): ParamInfo | undefined;
  };

  /** Call a named Rust handler. Returns a Promise with the result. */
  invoke(method: string, ...args: unknown[]): Promise<unknown>;

  /** Listen for named events from Rust. Returns unsubscribe function. */
  on(event: string, callback: (data: unknown) => void): () => void;

  /** Send a named event to Rust. */
  emit(event: string, data?: unknown): void;
}

interface ParamInfo {
  id: number;          // numeric ParameterId
  stringId: string;    // from #[parameter(id = "...")]
  name: string;        // display name
  value: number;       // normalized (0-1)
  defaultValue: number; // normalized (0-1)
  min: number;         // plain range min (e.g. -60 for dB)
  max: number;         // plain range max (e.g. 12 for dB)
  units: string;       // "dB", "Hz", etc.
  steps: number;       // 0 = continuous
}
```

### Usage Example (React)

```tsx
import { useEffect, useState } from "react";

declare const __BEAMER__: Beamer;

function GainSlider() {
  const [gain, setGain] = useState(0.5);

  useEffect(() => {
    // Wait for parameter init
    __BEAMER__.ready.then(() => {
      setGain(__BEAMER__.params.get("gain"));
    });

    // Subscribe to host automation changes
    return __BEAMER__.params.on("gain", setGain);
  }, []);

  const handleChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const v = parseFloat(e.target.value);
    setGain(v);
    __BEAMER__.params.set("gain", v);
  };

  return (
    <input
      type="range" min={0} max={1} step={0.001}
      value={gain}
      onMouseDown={() => __BEAMER__.params.beginEdit("gain")}
      onChange={handleChange}
      onMouseUp={() => __BEAMER__.params.endEdit("gain")}
    />
  );
}
```

## Injected Runtime Script

A `WKUserScript` injected at document start before any page code runs.
This script creates the `window.__BEAMER__` object and its internal
plumbing:

```javascript
(function() {
  var paramMap = {};     // stringId -> {id, value, listeners}
  var paramById = {};    // numericId -> paramMap entry
  var pendingParamSubs = {}; // stringId -> [cb, ...] (before _onInit)
  var eventListeners = {};
  var invokeCallbacks = {};
  var nextCallId = 0;
  var readyResolve;
  var readyPromise = new Promise(function(r) { readyResolve = r; });

  function post(msg) {
    window.webkit.messageHandlers.beamer.postMessage(msg);
  }

  window.__BEAMER__ = {
    ready: readyPromise,

    params: {
      get: function(stringId) {
        var p = paramMap[stringId];
        return p ? p.value : 0;
      },
      set: function(stringId, value) {
        var p = paramMap[stringId];
        if (!p) return;
        p.value = value;
        post({type:"param:set", id:p.id, value:value});
      },
      beginEdit: function(stringId) {
        var p = paramMap[stringId];
        if (p) post({type:"param:begin", id:p.id});
      },
      endEdit: function(stringId) {
        var p = paramMap[stringId];
        if (p) post({type:"param:end", id:p.id});
      },
      on: function(stringId, cb) {
        var p = paramMap[stringId];
        if (!p) {
          // Queue subscription until _onInit populates the param map.
          if (!pendingParamSubs[stringId]) pendingParamSubs[stringId] = [];
          pendingParamSubs[stringId].push(cb);
          return function() {
            var arr = pendingParamSubs[stringId];
            if (arr) {
              pendingParamSubs[stringId] = arr.filter(function(f){return f!==cb;});
              return;
            }
            var q = paramMap[stringId];
            if (q) q.listeners = q.listeners.filter(function(f){return f!==cb;});
          };
        }
        p.listeners.push(cb);
        return function() {
          p.listeners = p.listeners.filter(function(f){return f!==cb;});
        };
      },
      all: function() {
        return Object.values(paramMap).map(function(p) { return p.info; });
      },
      info: function(stringId) {
        var p = paramMap[stringId];
        return p ? p.info : undefined;
      }
    },

    invoke: function(method) {
      var args = Array.prototype.slice.call(arguments, 1);
      return new Promise(function(resolve, reject) {
        var id = nextCallId++;
        invokeCallbacks[id] = {resolve: resolve, reject: reject};
        post({type:"invoke", method:method, args:args, callId:id});
      });
    },

    on: function(name, cb) {
      if (!eventListeners[name]) eventListeners[name] = [];
      eventListeners[name].push(cb);
      return function() {
        eventListeners[name] = eventListeners[name]
          .filter(function(f){return f!==cb;});
      };
    },

    emit: function(name, data) {
      post({type:"event", name:name, data:data});
    },

    // --- Internal callbacks (called by native evaluateJavaScript) ---

    _onInit: function(params) {
      params.forEach(function(p) {
        var pending = pendingParamSubs[p.stringId] || [];
        delete pendingParamSubs[p.stringId];
        var entry = {
          id: p.id, value: p.value, listeners: pending,
          info: p
        };
        paramMap[p.stringId] = entry;
        paramById[p.id] = entry;
      });
      readyResolve();
    },

    _onParams: function(changed) {
      for (var id in changed) {
        var entry = paramById[id];
        if (entry) {
          entry.value = changed[id];
          entry.info.value = changed[id];
          entry.listeners.forEach(function(cb) { cb(entry.value); });
        }
      }
    },

    _onResult: function(callId, result) {
      var cb = invokeCallbacks[callId];
      if (!cb) return;
      delete invokeCallbacks[callId];
      if (result && result.err !== undefined) {
        cb.reject(result.err);
      } else {
        cb.resolve(result ? result.ok : null);
      }
    },

    _onEvent: function(name, data) {
      var cbs = eventListeners[name];
      if (cbs) cbs.forEach(function(cb) { cb(data); });
    }
  };
})();
```

Notes:
- Uses `var` and `function` (no ES6) for maximum WebView compatibility
- The `post` helper calls `webkit.messageHandlers.beamer.postMessage`
  directly (macOS). On Windows (future) this would use
  `window.chrome.webview.postMessage`.
- The `_onInit`, `_onParams`, `_onResult` and `_onEvent` functions are
  called by native code via `evaluateJavaScript`.
- `params.on()` can be called before `_onInit` (before `ready`
  resolves). Subscriptions are queued in `pendingParamSubs` and flushed
  into the parameter entry's listener list when `_onInit` runs. The
  returned unsubscribe function works in both states.
- `invoke()` Promises reject when the Rust handler returns an error.
  Native code sends `_onResult(id, {"err":"..."})` for errors and
  `_onResult(id, {"ok":...})` for success.

## Platform Implementation (macOS)

### WKScriptMessageHandler

A new ObjC class `BeamerMessageHandler` receives messages from
JavaScript. Like the scheme handler, it uses `ClassBuilder` for
runtime class construction:

```rust
// beamer-webview/src/platform/macos_ipc.rs

/// Callback signature for messages from JavaScript.
pub type MessageCallback =
    unsafe extern "C-unwind" fn(context: *mut c_void, json: *const c_char, len: usize);

/// Ivar: the callback function pointer.
const CALLBACK_IVAR: &CStr = c"_beamerCallback";
/// Ivar: the callback context pointer.
const CONTEXT_IVAR: &CStr = c"_beamerContext";
```

The handler class conforms to `WKScriptMessageHandler` and implements
`userContentController:didReceiveScriptMessage:`:

1. Extract `message.body` as `NSString`
2. Convert to UTF-8 `*const c_char`
3. Call the stored function pointer with context and JSON

Only one ObjC class is needed (`BeamerMessageHandler`). Unlike the
scheme handler, the message handler is added per-WKUserContentController
(per-WebView), so there are no process-wide name collisions. If the
class already exists (plugin reopened), reuse it.

### Registration

In `MacosWebView::attach_to_parent`, after creating the
`WKWebViewConfiguration`:

```rust
// Inject the __BEAMER__ runtime script
let user_script = WKUserScript::initWithSource_injectionTime_forMainFrameOnly(
    mtm.alloc(),
    &NSString::from_str(BEAMER_RUNTIME_JS),
    WKUserScriptInjectionTimeAtDocumentStart,
    true,
);
let content_controller = unsafe { wk_config.userContentController() };
content_controller.addUserScript(&user_script);

// Register message handler
let handler = new_message_handler(callback, context);
content_controller.addScriptMessageHandler_name(&handler, &NSString::from_str("beamer"));
```

### Cleanup

When the WebView is detached, the message handler must be removed from
the `WKUserContentController` to break the retain cycle (controller
retains handler, handler holds the context pointer):

```rust
// In MacosWebView::detach(), before removeFromSuperview:
let content_controller = unsafe { self.webview.configuration().userContentController() };
content_controller.removeScriptMessageHandlerForName(&NSString::from_str("beamer"));
content_controller.removeAllUserScripts();
```

The same cleanup applies in the AU wrapper templates (`viewDidDisappear`
/ `dealloc`) before calling `beamer_webview_destroy`. A C-ABI helper
can encapsulate this if needed:

```rust
#[no_mangle]
pub extern "C" fn beamer_webview_remove_message_handler(handle: *mut c_void);
```

### evaluateJavaScript

New method on `MacosWebView`:

```rust
impl MacosWebView {
    /// Evaluate JavaScript in the WebView.
    ///
    /// Must be called from the main thread. The completion handler is
    /// optional (pass null for fire-and-forget).
    pub fn evaluate_js(&self, script: &str) {
        let ns_script = NSString::from_str(script);
        unsafe {
            self.webview.evaluateJavaScript_completionHandler(
                &ns_script,
                None,
            );
        }
    }
}
```

### C-ABI Additions

```rust
// beamer-webview/src/ffi.rs

/// Evaluate JavaScript in the WebView.
///
/// # Safety
/// - handle must be a valid pointer from beamer_webview_create
/// - script must be a valid null-terminated UTF-8 C string
/// - Must be called from the main thread
#[no_mangle]
pub extern "C" fn beamer_webview_eval_js(
    handle: *mut c_void,
    script: *const c_char,
);

/// Set the message callback for JS -> native communication.
///
/// The callback fires on the main thread when JavaScript calls
/// `window.__BEAMER__.emit()`, `invoke()`, or `params.set()`.
///
/// # Safety
/// - handle must be a valid pointer from beamer_webview_create
/// - callback must be a valid function pointer
/// - context must remain valid until the WebView is destroyed
#[no_mangle]
pub extern "C" fn beamer_webview_set_message_callback(
    handle: *mut c_void,
    callback: MessageCallback,
    context: *mut c_void,
);
```

Alternative: pass the callback and context at creation time via
extended `beamer_webview_create` signatures. This avoids a separate
setup call and ensures the callback is ready before any JS runs.

### WKNavigationDelegate

Register a navigation delegate to detect when the initial page load
completes. On `webView:didFinishNavigation:`, fire a callback so the
format wrapper can send the parameter init dump.

```rust
/// Callback fired when the WebView finishes loading initial content.
pub type LoadedCallback =
    unsafe extern "C-unwind" fn(context: *mut c_void);
```

## Parameter Synchronization

### Init Dump

When the WebView finishes loading (navigation delegate callback), the
format wrapper serializes all parameter info and sends it via
`evaluateJavaScript`:

```javascript
window.__BEAMER__._onInit([
  {"id":42,"stringId":"gain","name":"Gain","min":-60,"max":12,
   "defaultValue":0.5,"value":0.75,"units":"dB","steps":0},
  ...
])
```

This resolves the `__BEAMER__.ready` promise and populates the
internal parameter map.

### String IDs

The `#[parameter(id = "gain")]` string is currently hashed to a
numeric `ParameterId` and discarded. Phase 2C adds a `string_id` field
to `ParameterInfo` so the string is available at runtime for the init
dump:

```rust
// beamer-core/src/parameter_info.rs
pub struct ParameterInfo {
    // ... existing fields ...
    /// Original string identifier from #[parameter(id = "...")].
    /// Empty string for parameters defined without a string ID.
    pub string_id: &'static str,
}
```

The `#[derive(Parameters)]` macro populates this from the attribute.

### Polling Timer

A 60 Hz timer on the main thread polls parameter values and pushes
changes to JavaScript. The timer:

1. Reads each parameter's normalized value (atomic load, lock-free)
2. Compares with the last value sent to JS
3. If any changed, batches them into one `_onParams` call
4. Updates the cached values

The timer lives in the format wrapper (not in `beamer-webview`)
because parameter access is format-specific:

- **VST3**: Reads from the shared `ParameterStore` directly (Rust)
- **AU**: Reads from the `AUParameterTree` (ObjC) or via C-ABI bridge

#### VST3 Timer

`WebViewPlugView` starts an `NSTimer` when `attached()` is called and
invalidates it in `removed()`:

```rust
// Pseudocode
fn start_sync_timer(&self) {
    // Schedule 60 Hz repeating timer on current run loop
    let timer = NSTimer::scheduledTimerWithTimeInterval_repeats(
        1.0 / 60.0, true, |_| {
            self.poll_and_push_params();
        }
    );
}

fn poll_and_push_params(&self) {
    let mut changed = String::from("window.__BEAMER__._onParams({");
    let mut any = false;

    for i in 0..self.params.count() {
        let info = self.params.info(i).unwrap();
        let val = self.params.get_normalized(info.id);
        if val != self.last_values[i] {
            self.last_values[i] = val;
            if any { changed.push(','); }
            write!(changed, "{}:{}", info.id, val);
            any = true;
        }
    }

    if any {
        changed.push_str("})");
        self.webview.evaluate_js(&changed);
    }
}
```

#### AU Timer

The generated ObjC wrapper starts an `NSTimer` in `loadView` (or
`_ensureWebView`) and polls via `beamer_au_param_get_normalized`:

```objc
_syncTimer = [NSTimer scheduledTimerWithTimeInterval:1.0/60.0
    repeats:YES block:^(NSTimer* t) {
    [self _pollParams];
}];
```

### UI -> Host Notification

When JavaScript sets a parameter value, the message reaches the format
wrapper's callback. The wrapper must notify the host:

**VST3:**
```rust
// On param:begin
handler.beginEdit(param_id);
// On param:set
params.set_normalized(param_id, value);
handler.performEdit(param_id, value);
// On param:end
handler.endEdit(param_id);
```

The `IComponentHandler` reference is shared between `Vst3Processor`
and `WebViewPlugView` via an `Arc<AtomicPtr<IComponentHandler>>`.

**AU (AUv3):**
The ObjC wrapper finds the `AUParameter` in the parameter tree and
sets its value. The AU framework handles host notification
automatically:

```objc
AUParameter* param = [self.audioUnit.parameterTree
    parameterWithAddress:paramId];
param.value = /* denormalized value */;
```

**AU (AUv2):**
The ObjC wrapper calls `AudioUnitSetParameter` which notifies the
host:

```objc
AudioUnitSetParameter(audioUnit, paramId,
    kAudioUnitScope_Global, 0, value, 0);
```

## Rust Plugin API

### Automatic Parameter Sync

For plugins that only need parameter binding, no additional code is
required. The framework handles sync automatically based on the
existing `#[derive(Parameters)]` and Config.

### Custom Handlers (Invoke + Events)

Plugins that need custom JS -> Rust communication implement a trait:

```rust
// beamer-core/src/webview_handler.rs

/// Handler for custom WebView messages.
///
/// Implement this to handle `invoke()` calls and custom events from
/// JavaScript. Parameter sync is handled automatically and does not
/// require this trait.
pub trait WebViewHandler: Send + Sync {
    /// Handle an invoke call from JavaScript.
    ///
    /// Called on the main thread when JS calls
    /// `__BEAMER__.invoke("method", args...)`.
    /// Return `Ok(value)` to resolve the JS Promise.
    /// Return `Err(message)` to reject the JS Promise.
    fn on_invoke(
        &self,
        _method: &str,
        _args: &[serde_json::Value],
    ) -> Result<serde_json::Value, String> {
        Ok(serde_json::Value::Null)
    }

    /// Handle a custom event from JavaScript.
    ///
    /// Called on the main thread when JS calls
    /// `__BEAMER__.emit("name", data)`.
    fn on_event(&self, _name: &str, _data: &serde_json::Value) {}
}
```

**Note on `serde_json`:** This is the standard JSON crate in the Rust
ecosystem. It adds a compile-time cost but the runtime is negligible.
Alternative: use a lightweight `JsonValue` enum defined in
`beamer-core` to avoid the dependency. Decision deferred to
implementation.

### Emitting Events from Rust

The `WebViewHandle` allows the plugin to push events to JavaScript:

```rust
// beamer-core/src/webview_handle.rs

/// Handle for sending events from Rust to the WebView.
///
/// Obtained via the processor context or GUI delegate. The handle is
/// `Send + Sync` and can be used from non-realtime threads (calls are
/// dispatched to the main thread internally).
pub struct WebViewHandle { /* ... */ }

impl WebViewHandle {
    /// Emit a named event to JavaScript.
    ///
    /// The event is delivered asynchronously. If the WebView is not
    /// attached, the call is silently dropped.
    ///
    /// **Not audio-thread safe.** This method allocates (JSON
    /// serialization + dispatch_async). For sending visualization
    /// data from the audio thread, see Phase 2D's lock-free ring
    /// buffer approach.
    pub fn emit(&self, name: &str, data: &impl serde::Serialize) {
        // Serialize to JSON, dispatch to main thread, evaluateJavaScript
    }
}
```

Thread safety: `emit()` can be called from any non-realtime thread. It
serializes the data and dispatches the `evaluateJavaScript` call to
the main thread via `dispatch_async(dispatch_get_main_queue(), ...)`.

**Audio-thread visualization** (spectrum data, level meters, etc.)
requires a lock-free path. This is deferred to Phase 2D, which adds a
ring buffer between the audio thread and the main-thread sync timer.
The timer drains the buffer and pushes updates to JavaScript alongside
parameter changes.

## Changes

### beamer-core

- [ ] Add `string_id: &'static str` to `ParameterInfo`
- [ ] Add `WebViewHandler` trait (with default no-op methods)
- [ ] Add `WebViewHandle` struct for Rust -> JS events

### beamer-macros

- [ ] Populate `string_id` from `#[parameter(id = "...")]` in the
  `#[derive(Parameters)]` expansion

### beamer-webview

- [ ] Add `macos_ipc.rs`: `BeamerMessageHandler` class via
  `ClassBuilder`, `WKScriptMessageHandler` conformance
- [ ] Add injected runtime script (`BEAMER_RUNTIME_JS` constant)
- [ ] Add `evaluate_js()` method to `MacosWebView`
- [ ] Add `WKNavigationDelegate` for load-complete detection
- [ ] Inject user script and register message handler in
  `attach_to_parent`
- [ ] Add `beamer_webview_eval_js` C-ABI export
- [ ] Update `WebViewConfig` with callback fields (message callback,
  loaded callback, context pointer)
- [ ] Add `WKUserScript`, `WKUserContentController` features to
  `objc2-web-kit` dependency
- [ ] Add `WKScriptMessage`, `WKScriptMessageHandler`,
  `WKNavigationDelegate`, `WKNavigation` features
- [ ] Remove message handler and user scripts in `detach()`
  (`removeScriptMessageHandlerForName:`, `removeAllUserScripts`)

### beamer-vst3

- [ ] Share `IComponentHandler` between `Vst3Processor` and
  `WebViewPlugView` via `Arc<AtomicPtr<IComponentHandler>>`
- [ ] Pass `ParameterStore` reference to `WebViewPlugView`
- [ ] Add message callback in `WebViewPlugView` that routes
  param:set/begin/end to host via `IComponentHandler`
- [ ] Add message callback routing for invoke and event (forwarded to
  `WebViewHandler` if present)
- [ ] Add `NSTimer`-based parameter sync (60 Hz) started in
  `attached()`, stopped in `removed()`
- [ ] Send init dump in navigation-complete callback
- [ ] Serialize parameter info to JSON for init dump

### beamer-au

- [ ] Add `beamer_au_param_count` C-ABI export
- [ ] Add `beamer_au_param_info_json` C-ABI export (returns JSON
  string with all parameter info for init dump)
- [ ] Add `beamer_au_param_get_normalized` C-ABI export
- [ ] Add `beamer_au_param_set_from_ui` C-ABI export (sets value and
  notifies host)

### AU wrappers (xtask codegen)

- [ ] AUv3: Add message callback that routes to parameter tree and
  invoke handler
- [ ] AUv3: Add `NSTimer` parameter sync (60 Hz) in view controller
- [ ] AUv3: Send init dump on load complete
- [ ] AUv2: Same changes in CocoaUI view factory
- [ ] Invalidate sync timer and clean up message handler before
  `beamer_webview_destroy`

### Example

- [ ] Update `webview-demo` to use parameter binding (replace local
  React state with `__BEAMER__.params`)
- [ ] Add invoke example (e.g., fetch plugin version from Rust)
- [ ] Add TypeScript type definitions file
  (`webview/src/beamer.d.ts`)
- [ ] Verify parameter sync in VST3 host
- [ ] Verify parameter sync in AU host
- [ ] Verify invoke round-trip
- [ ] Verify event emission

## TypeScript Definitions

A `.d.ts` file for plugin web projects:

```typescript
// beamer.d.ts

interface BeamerParamInfo {
  id: number;
  stringId: string;
  name: string;
  value: number; // normalized (0-1)
  defaultValue: number; // normalized (0-1)
  min: number; // plain range (e.g. -60 for dB)
  max: number; // plain range (e.g. 12 for dB)
  units: string;
  steps: number; // 0 = continuous
}

interface BeamerParams {
  get(stringId: string): number;
  set(stringId: string, value: number): void;
  beginEdit(stringId: string): void;
  endEdit(stringId: string): void;
  on(stringId: string, callback: (value: number) => void): () => void;
  all(): BeamerParamInfo[];
  info(stringId: string): BeamerParamInfo | undefined;
}

interface Beamer {
  readonly ready: Promise<void>;
  readonly params: BeamerParams;
  invoke(method: string, ...args: unknown[]): Promise<unknown>;
  on(event: string, callback: (data: unknown) => void): () => void;
  emit(event: string, data?: unknown): void;
}

declare const __BEAMER__: Beamer;
```

Plugin web projects reference this file in their `tsconfig.json`:

```json
{
  "compilerOptions": {
    "types": ["./src/beamer"]
  }
}
```

## Development Workflow

### Dev Server Mode

When using `BEAMER_DEV_URL`, the injected runtime script and message
handler are still active. IPC works identically in dev and production
modes because the transport (WKScriptMessageHandler /
evaluateJavaScript) is independent of the content source.

### Debugging

With dev tools enabled (`dev_tools: true`), developers can:
- Inspect `window.__BEAMER__` in the Safari Web Inspector console
- Monitor IPC messages via `console.log` in the message callback
- Check parameter state with `__BEAMER__.params.all()`

## Dependencies

New `objc2-web-kit` features:

```toml
objc2-web-kit = { version = "0.3", features = [
    # Existing
    "WKWebView", "WKWebViewConfiguration", "WKURLSchemeHandler",
    "WKURLSchemeTask",
    # New for Phase 2C
    "WKUserContentController", "WKUserScript",
    "WKScriptMessage", "WKScriptMessageHandler",
    "WKNavigationDelegate", "WKNavigation",
] }
```

New crate dependency (optional, see note in Rust Plugin API):

```toml
serde_json = "1"
serde = { version = "1", features = ["derive"] }
```

## References

- [WKScriptMessageHandler](https://developer.apple.com/documentation/webkit/wkscriptmessagehandler) -
  receiving JS messages in native code
- [WKUserScript](https://developer.apple.com/documentation/webkit/wkuserscript) -
  injecting JavaScript at document start
- [WKUserContentController](https://developer.apple.com/documentation/webkit/wkusercontentcontroller) -
  managing user scripts and message handlers
- [WKNavigationDelegate](https://developer.apple.com/documentation/webkit/wknavigationdelegate) -
  detecting page load completion
- [evaluateJavaScript](https://developer.apple.com/documentation/webkit/wkwebview/evaluatejavascript(_:completionhandler:)) -
  executing JS from native code

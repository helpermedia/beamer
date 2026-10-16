(function() {
  var paramMap = {};
  var paramById = {};
  var pendingParamSubs = {};
  // State fields by path: {value, json, listeners}. `json` is the value as
  // canonical text, to tell a change from a value the UI has already.
  var stateMap = {};
  var pendingStateSubs = {};
  var eventListeners = {};
  var invokeCallbacks = {};
  var nextCallId = 0;
  var readyResolve;
  var readyPromise = new Promise(function(r) { readyResolve = r; });

  // JSON.stringify is intentional: postMessage accepts any plist-compatible
  // type, but the native side expects a plain UTF-8 string so it can be
  // forwarded through the C-ABI callback as raw bytes. Passing an object
  // would give an NSDictionary, which is harder to bridge.
  var nativeHandler = window.webkit
    && window.webkit.messageHandlers
    && window.webkit.messageHandlers.beamer;

  function post(msg) {
    if (nativeHandler) nativeHandler.postMessage(JSON.stringify(msg));
  }

  // Call a listener of the UI. Native code delivers several changes and
  // events in one script, so a listener that throws must not stop the
  // delivery of what comes after it.
  function notify(cb, value) {
    try {
      cb(value);
    } catch (e) {
      console.error(e);
    }
  }

  // Subscribe to an entry of `entries`, or queue the listener in `pending`
  // until init brings the entry. Returns the function that unsubscribes.
  function subscribe(entries, pending, key, cb) {
    var entry = entries[key];
    if (!entry) {
      if (!pending[key]) pending[key] = [];
      pending[key].push(cb);
      return function() {
        var arr = pending[key];
        if (arr) {
          pending[key] = arr.filter(function(f){return f!==cb;});
          return;
        }
        var e = entries[key];
        if (e) e.listeners = e.listeners.filter(function(f){return f!==cb;});
      };
    }
    entry.listeners.push(cb);
    return function() {
      entry.listeners = entry.listeners.filter(function(f){return f!==cb;});
    };
  }

  // JSON text with the keys of every object sorted, so that two equal
  // values give the same text whatever order their keys came in (a Rust
  // map is written in another order each time).
  function canonical(value) {
    if (value === null || typeof value !== "object") return JSON.stringify(value);
    if (Array.isArray(value)) {
      return "[" + value.map(function(v) {
        var text = canonical(v);
        return text === undefined ? "null" : text;
      }).join(",") + "]";
    }
    var parts = [];
    Object.keys(value).sort().forEach(function(key) {
      var text = canonical(value[key]);
      if (text !== undefined) parts.push(JSON.stringify(key) + ":" + text);
    });
    return "{" + parts.join(",") + "}";
  }

  // Give a State field a new value and tell its listeners, unless the UI
  // has that value already.
  function applyState(path, value) {
    var entry = stateMap[path];
    if (!entry) return;
    var json = canonical(value);
    if (json === entry.json) return;
    entry.value = value;
    entry.json = json;
    entry.listeners.forEach(function(cb) { notify(cb, value); });
  }

  window.__BEAMER__ = {
    ready: readyPromise,

    // The plugin format the editor runs in: "vst3", "auv2" or "auv3". Set
    // before `ready` resolves.
    pluginFormat: null,

    params: {
      get: function(stringId) {
        var p = paramMap[stringId];
        return p ? p.value : 0;
      },
      getPlain: function(stringId) {
        var p = paramMap[stringId];
        return p ? p.plainValue : 0;
      },
      getDisplayText: function(stringId) {
        var p = paramMap[stringId];
        return p ? p.displayText : "";
      },
      set: function(stringId, value) {
        var p = paramMap[stringId];
        if (!p) return;
        p.value = value;
        p.info.value = value;
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
        return subscribe(paramMap, pendingParamSubs, stringId, cb);
      },
      all: function() {
        return Object.values(paramMap).map(function(p) { return p.info; });
      },
      info: function(stringId) {
        var p = paramMap[stringId];
        return p ? p.info : undefined;
      }
    },

    // The State fields of the plugin, by path ("pan_law", "layer/sample").
    // A value is what JSON makes of the Rust value.
    state: {
      // The value the UI knows: the same object until it changes.
      // Undefined before `ready` and for a path the plugin does not have.
      get: function(path) {
        var entry = stateMap[path];
        return entry ? entry.value : undefined;
      },
      // Set a value. Listeners hear of it at once; the plugin answers with
      // the value in effect, which corrects one that does not fit.
      set: function(path, value) {
        if (!stateMap[path]) return;
        // Keep the value as the plugin will see it: `undefined` becomes
        // null and so does NaN.
        var plain = JSON.parse(JSON.stringify(value === undefined ? null : value));
        applyState(path, plain);
        post({type:"state:set", id:path, value:plain});
      },
      on: function(path, cb) {
        return subscribe(stateMap, pendingStateSubs, path, cb);
      },
      // The paths of all State fields. Empty before `ready`.
      ids: function() {
        return Object.keys(stateMap);
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

    _onInit: function(params, info) {
      if (info && info.pluginFormat) {
        window.__BEAMER__.pluginFormat = info.pluginFormat;
      }
      params.forEach(function(p) {
        var pending = pendingParamSubs[p.stringId] || [];
        delete pendingParamSubs[p.stringId];
        var entry = {
          id: p.id, value: p.value, plainValue: p.plainValue,
          displayText: p.displayText, listeners: pending, info: p
        };
        paramMap[p.stringId] = entry;
        paramById[p.id] = entry;
        // A listener that subscribed before the value existed hears it now.
        pending.forEach(function(cb) { notify(cb, p.value); });
      });
      var state = (info && info.state) || {};
      Object.keys(state).forEach(function(path) {
        var pending = pendingStateSubs[path] || [];
        delete pendingStateSubs[path];
        stateMap[path] = {
          value: state[path], json: canonical(state[path]), listeners: pending
        };
        pending.forEach(function(cb) { notify(cb, state[path]); });
      });
      readyResolve();
    },

    _onState: function(changed) {
      for (var path in changed) applyState(path, changed[path]);
    },

    _onParams: function(changed) {
      for (var id in changed) {
        var entry = paramById[id];
        if (entry) {
          var tuple = changed[id];
          entry.value = tuple[0];
          entry.plainValue = tuple[1];
          entry.displayText = tuple[2];
          entry.info.value = tuple[0];
          entry.info.plainValue = tuple[1];
          entry.info.displayText = tuple[2];
          entry.listeners.forEach(function(cb) { notify(cb, entry.value); });
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
      if (cbs) cbs.forEach(function(cb) { notify(cb, data); });
    }
  };
})();
